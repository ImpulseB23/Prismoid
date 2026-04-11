//! SPSC shared memory ring buffer for the Go sidecar → Rust host hot path.
//!
//! The Rust host creates an unnamed shared memory section and hands the raw
//! HANDLE to the Go sidecar at spawn time via handle inheritance + stdio
//! bootstrap. See ADR 18 (revised 2026-04-11). Windows is the primary target;
//! Linux and macOS return `ErrorKind::Unsupported` until their own tickets land.

use std::io;
use std::sync::atomic::{AtomicU64, Ordering};

const CACHE_LINE: usize = 64;
const HEADER_SIZE: usize = CACHE_LINE * 3;

pub const DEFAULT_CAPACITY: usize = 4 * 1024 * 1024; // 4MB ring data

/// Portable integer representation of a platform shared memory handle.
/// On Windows this is a `HANDLE` cast through `usize`. On POSIX platforms this
/// will be a file descriptor packed into the same width.
pub type RawHandle = usize;

#[repr(C, align(64))]
struct WriteSlot {
    index: AtomicU64,
    _pad: [u8; CACHE_LINE - 8],
}

#[repr(C, align(64))]
struct ReadSlot {
    index: AtomicU64,
    _pad: [u8; CACHE_LINE - 8],
}

#[repr(C, align(64))]
struct MetaSlot {
    capacity: u64,
    _pad: [u8; CACHE_LINE - 8],
}

#[derive(Debug)]
pub struct RingBufReader {
    base: *mut u8,
    map_size: usize,
    data_offset: usize,
    #[cfg(windows)]
    mapping_handle: windows::Win32::Foundation::HANDLE,
    owner: bool,
}

// The reader owns a view into shared memory and is the sole consumer. Atomics
// in the header enforce cross-process ordering, and the struct only holds raw
// pointers into memory that is valid for the reader's lifetime.
unsafe impl Send for RingBufReader {}

#[cfg(windows)]
impl RingBufReader {
    pub fn create_owner(capacity: usize) -> io::Result<Self> {
        use windows::core::PCWSTR;
        use windows::Win32::Foundation::{CloseHandle, INVALID_HANDLE_VALUE};
        use windows::Win32::System::Memory::{
            CreateFileMappingW, MapViewOfFile, FILE_MAP, FILE_MAP_READ, FILE_MAP_WRITE,
            PAGE_READWRITE,
        };

        if capacity < 4 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "ring buffer capacity must be at least 4 bytes",
            ));
        }

        let total = HEADER_SIZE
            .checked_add(capacity)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "capacity overflow"))?;
        let hi = ((total as u64 >> 32) & 0xFFFF_FFFF) as u32;
        let lo = (total as u64 & 0xFFFF_FFFF) as u32;

        let handle = unsafe {
            CreateFileMappingW(
                INVALID_HANDLE_VALUE,
                None,
                PAGE_READWRITE,
                hi,
                lo,
                PCWSTR::null(),
            )
        }
        .map_err(windows_err)?;

        let view = unsafe {
            MapViewOfFile(
                handle,
                FILE_MAP(FILE_MAP_READ.0 | FILE_MAP_WRITE.0),
                0,
                0,
                total,
            )
        };
        if view.Value.is_null() {
            unsafe {
                let _ = CloseHandle(handle);
            }
            return Err(io::Error::other("MapViewOfFile returned null"));
        }

        let base = view.Value as *mut u8;

        unsafe {
            std::ptr::write_bytes(base, 0, total);
            let meta = &mut *(base.add(CACHE_LINE * 2) as *mut MetaSlot);
            meta.capacity = capacity as u64;
        }

        Ok(Self {
            base,
            map_size: total,
            data_offset: HEADER_SIZE,
            mapping_handle: handle,
            owner: true,
        })
    }

    pub fn attach(handle: RawHandle, map_size: usize) -> io::Result<Self> {
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::System::Memory::{
            MapViewOfFile, FILE_MAP, FILE_MAP_READ, FILE_MAP_WRITE,
        };

        if map_size < HEADER_SIZE + 4 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "map_size too small for ring buffer header + minimum data",
            ));
        }

        let mapping_handle = HANDLE(handle as *mut _);
        let view = unsafe {
            MapViewOfFile(
                mapping_handle,
                FILE_MAP(FILE_MAP_READ.0 | FILE_MAP_WRITE.0),
                0,
                0,
                map_size,
            )
        };
        if view.Value.is_null() {
            return Err(io::Error::last_os_error());
        }

        Ok(Self {
            base: view.Value as *mut u8,
            map_size,
            data_offset: HEADER_SIZE,
            mapping_handle,
            owner: false,
        })
    }

    /// Raw handle suitable for passing to a child process via stdio bootstrap.
    /// Only meaningful for readers created via `create_owner`.
    pub fn raw_handle(&self) -> RawHandle {
        self.mapping_handle.0 as RawHandle
    }
}

#[cfg(not(windows))]
impl RingBufReader {
    pub fn create_owner(_capacity: usize) -> io::Result<Self> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "ring buffer not yet supported on this platform",
        ))
    }

    pub fn attach(_handle: RawHandle, _map_size: usize) -> io::Result<Self> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "ring buffer not yet supported on this platform",
        ))
    }

    pub fn raw_handle(&self) -> RawHandle {
        0
    }
}

impl RingBufReader {
    pub fn map_size(&self) -> usize {
        self.map_size
    }

    fn header(&self) -> (*const WriteSlot, *const ReadSlot, u64) {
        unsafe {
            let write_slot = self.base as *const WriteSlot;
            let read_slot = self.base.add(CACHE_LINE) as *const ReadSlot;
            let meta = &*(self.base.add(CACHE_LINE * 2) as *const MetaSlot);
            (write_slot, read_slot, meta.capacity)
        }
    }

    fn data_ptr(&self) -> *const u8 {
        unsafe { self.base.add(self.data_offset) }
    }

    pub fn drain(&mut self) -> Vec<Vec<u8>> {
        let (write_slot, read_slot, capacity) = self.header();
        let cap = capacity as usize;
        let data = self.data_ptr();

        let mut messages = Vec::new();

        unsafe {
            let write_pos = (*write_slot).index.load(Ordering::Acquire) as usize;
            let mut read_pos = (*read_slot).index.load(Ordering::Relaxed) as usize;

            while read_pos + 4 <= write_pos {
                let len = self.read_u32_wrapped(data, read_pos, cap);
                let msg_len = len as usize;

                if msg_len > cap {
                    tracing::error!(
                        msg_len,
                        cap,
                        "corrupt ring buffer: msg_len exceeds capacity"
                    );
                    break;
                }

                if read_pos + 4 + msg_len > write_pos {
                    break;
                }

                let mut msg = vec![0u8; msg_len];
                self.read_wrapped(data, read_pos + 4, cap, &mut msg);

                messages.push(msg);
                read_pos += 4 + msg_len;
            }

            (*read_slot).index.store(read_pos as u64, Ordering::Release);
        }

        messages
    }

    unsafe fn read_u32_wrapped(&self, data: *const u8, pos: usize, cap: usize) -> u32 {
        let mut buf = [0u8; 4];
        unsafe {
            self.read_wrapped(data, pos, cap, &mut buf);
        }
        u32::from_be_bytes(buf)
    }

    unsafe fn read_wrapped(&self, data: *const u8, pos: usize, cap: usize, out: &mut [u8]) {
        let offset = pos % cap;
        let first_chunk = cap - offset;

        unsafe {
            if first_chunk >= out.len() {
                std::ptr::copy_nonoverlapping(data.add(offset), out.as_mut_ptr(), out.len());
            } else {
                std::ptr::copy_nonoverlapping(data.add(offset), out.as_mut_ptr(), first_chunk);
                std::ptr::copy_nonoverlapping(
                    data,
                    out.as_mut_ptr().add(first_chunk),
                    out.len() - first_chunk,
                );
            }
        }
    }
}

impl Drop for RingBufReader {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            use windows::Win32::Foundation::CloseHandle;
            use windows::Win32::System::Memory::{UnmapViewOfFile, MEMORY_MAPPED_VIEW_ADDRESS};

            if !self.base.is_null() {
                let addr = MEMORY_MAPPED_VIEW_ADDRESS {
                    Value: self.base as *mut _,
                };
                let _ = UnmapViewOfFile(addr);
            }
            if self.owner {
                let _ = CloseHandle(self.mapping_handle);
            }
        }
    }
}

#[cfg(windows)]
fn windows_err(err: windows::core::Error) -> io::Error {
    io::Error::from_raw_os_error(err.code().0)
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    fn write_to_buf(reader: &RingBufReader, payloads: &[&[u8]]) {
        let (write_slot, _, capacity) = reader.header();
        let cap = capacity as usize;
        let data = unsafe { reader.base.add(HEADER_SIZE) };

        unsafe {
            let mut write_pos = (*write_slot).index.load(Ordering::Relaxed) as usize;
            for payload in payloads {
                let len_bytes = (payload.len() as u32).to_be_bytes();
                let offset = write_pos % cap;
                let first_chunk = cap - offset;

                if first_chunk >= 4 {
                    std::ptr::copy_nonoverlapping(len_bytes.as_ptr(), data.add(offset), 4);
                } else {
                    std::ptr::copy_nonoverlapping(
                        len_bytes.as_ptr(),
                        data.add(offset),
                        first_chunk,
                    );
                    std::ptr::copy_nonoverlapping(
                        len_bytes.as_ptr().add(first_chunk),
                        data,
                        4 - first_chunk,
                    );
                }

                let pay_offset = (write_pos + 4) % cap;
                let pay_first = cap - pay_offset;
                if pay_first >= payload.len() {
                    std::ptr::copy_nonoverlapping(
                        payload.as_ptr(),
                        data.add(pay_offset),
                        payload.len(),
                    );
                } else {
                    std::ptr::copy_nonoverlapping(
                        payload.as_ptr(),
                        data.add(pay_offset),
                        pay_first,
                    );
                    std::ptr::copy_nonoverlapping(
                        payload.as_ptr().add(pay_first),
                        data,
                        payload.len() - pay_first,
                    );
                }

                write_pos += 4 + payload.len();
            }
            (*write_slot)
                .index
                .store(write_pos as u64, Ordering::Release);
        }
    }

    #[test]
    fn rejects_capacity_too_small() {
        let err = RingBufReader::create_owner(2).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn create_default_capacity() {
        let reader = RingBufReader::create_owner(DEFAULT_CAPACITY).unwrap();
        assert!(reader.map_size() >= DEFAULT_CAPACITY + HEADER_SIZE);
        assert!(reader.raw_handle() != 0);
    }

    #[test]
    fn create_and_drain_empty() {
        let mut reader = RingBufReader::create_owner(4096).unwrap();
        let messages = reader.drain();
        assert!(messages.is_empty());
    }

    #[test]
    fn write_and_read_single_message() {
        let mut reader = RingBufReader::create_owner(4096).unwrap();
        write_to_buf(&reader, &[b"hello world"]);

        let messages = reader.drain();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0], b"hello world");
    }

    #[test]
    fn write_and_read_multiple_messages() {
        let mut reader = RingBufReader::create_owner(4096).unwrap();
        write_to_buf(&reader, &[b"msg1", b"msg two", b"third message"]);

        let messages = reader.drain();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0], b"msg1");
        assert_eq!(messages[1], b"msg two");
        assert_eq!(messages[2], b"third message");
    }

    #[test]
    fn drain_stops_on_corrupt_length() {
        let mut reader = RingBufReader::create_owner(256).unwrap();
        let (write_slot, _, _) = reader.header();
        let data = unsafe { reader.base.add(HEADER_SIZE) };

        let bad_len: u32 = 257;
        let len_bytes = bad_len.to_be_bytes();

        unsafe {
            std::ptr::copy_nonoverlapping(len_bytes.as_ptr(), data, 4);
            (*write_slot)
                .index
                .store(4 + bad_len as u64, Ordering::Release);
        }

        let messages = reader.drain();
        assert!(messages.is_empty());
    }

    #[test]
    fn drain_stops_on_partial_message() {
        let mut reader = RingBufReader::create_owner(4096).unwrap();
        let (write_slot, _, _) = reader.header();
        let data = unsafe { reader.base.add(HEADER_SIZE) };

        let len_bytes = 100u32.to_be_bytes();

        unsafe {
            std::ptr::copy_nonoverlapping(len_bytes.as_ptr(), data, 4);
            (*write_slot).index.store(54, Ordering::Release);
        }

        let messages = reader.drain();
        assert!(messages.is_empty());
    }

    #[test]
    fn read_wraps_around_boundary() {
        let mut reader = RingBufReader::create_owner(32).unwrap();
        let (write_slot, read_slot, _) = reader.header();

        unsafe {
            (*write_slot).index.store(24, Ordering::Release);
            (*read_slot).index.store(24, Ordering::Release);
        }

        write_to_buf(&reader, &[b"ABCDEFGH"]);

        let messages = reader.drain();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0], b"ABCDEFGH");
    }

    #[test]
    fn attach_reads_through_owner_handle() {
        // In-process exercise of the cross-process handle contract: create an
        // owner, then open a second view on the same section via attach using
        // the raw handle. Messages written via the owner's view are visible
        // through the attached reader's view. The kernel section stays alive
        // as long as the owner holds its handle.
        let owner = RingBufReader::create_owner(4096).unwrap();
        let handle = owner.raw_handle();
        let size = owner.map_size();

        let mut attached = RingBufReader::attach(handle, size).unwrap();
        write_to_buf(&owner, &[b"cross-view message"]);

        let messages = attached.drain();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0], b"cross-view message");
    }
}
