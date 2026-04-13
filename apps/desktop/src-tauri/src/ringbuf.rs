//! SPSC shared memory ring buffer for the Go sidecar → Rust host hot path.
//!
//! The Rust host creates an unnamed shared memory section via [`RingBufReader::create_owner`]
//! and obtains the raw HANDLE via [`RingBufReader::raw_handle`]. See ADR 18
//! (revised 2026-04-11).
//!
//! Alongside the section, `create_owner` also creates an unnamed auto-reset
//! Windows Event that the Go sidecar signals after each successful ring write
//! (via `SetEvent`). The host parks on [`RingBufReader::wait_for_signal`] to
//! wake the instant new data is available, eliminating per-frame polling
//! latency. This matches the industry-standard pattern used by Chromium Mojo
//! and LMAX Disruptor for low-latency shm IPC. A caller-provided timeout acts
//! as a belt-and-suspenders fallback in case a signal is ever lost.
//!
//! This primitive deliberately creates both handles as **non-inheritable**.
//! The caller (the host lifecycle) is responsible for marking them inheritable
//! via `SetHandleInformation(HANDLE_FLAG_INHERIT)` immediately before spawning
//! the sidecar and un-marking them immediately after, to minimize the race
//! window where any other child spawned in that interval would inherit them.
//! See the Rust stdlib comment in `library/std/src/sys/process/windows.rs`
//! (around the `CREATE_PROCESS_LOCK` definition) for why this window matters.
//!
//! Windows is the primary target; Linux and macOS return `ErrorKind::Unsupported`
//! until their own tickets land.

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
    /// Auto-reset event signaled by the writer after each ring write. Only set
    /// for owner-created readers; attach-created readers have no event.
    #[cfg(windows)]
    event_handle: Option<windows::Win32::Foundation::HANDLE>,
    owner: bool,
}

/// Result of [`RingBufReader::wait_for_signal`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WaitOutcome {
    /// The writer signaled the event; new data is available.
    Signaled,
    /// The timeout elapsed with no signal. Fallback path; drain anyway.
    TimedOut,
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
            CreateFileMappingW, MapViewOfFile, UnmapViewOfFile, FILE_MAP, FILE_MAP_READ,
            FILE_MAP_WRITE, MEMORY_MAPPED_VIEW_ADDRESS, PAGE_READWRITE,
        };
        use windows::Win32::System::Threading::CreateEventW;

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
            let err = io::Error::last_os_error();
            unsafe {
                let _ = CloseHandle(handle);
            }
            return Err(err);
        }

        let base = view.Value as *mut u8;

        unsafe {
            std::ptr::write_bytes(base, 0, total);
            let meta = &mut *(base.add(CACHE_LINE * 2) as *mut MetaSlot);
            meta.capacity = capacity as u64;
        }

        // Auto-reset unnamed event; starts unsignaled. The writer (Go sidecar)
        // calls SetEvent after each ring write. The reader waits on it via
        // WaitForSingleObject to wake immediately when data is available.
        let event = match unsafe { CreateEventW(None, false, false, PCWSTR::null()) } {
            Ok(h) => h,
            Err(e) => {
                let err = windows_err(e);
                unsafe {
                    let _ = UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS {
                        Value: base as *mut _,
                    });
                    let _ = CloseHandle(handle);
                }
                return Err(err);
            }
        };

        Ok(Self {
            base,
            map_size: total,
            data_offset: HEADER_SIZE,
            mapping_handle: handle,
            event_handle: Some(event),
            owner: true,
        })
    }

    pub fn attach(handle: RawHandle, map_size: usize) -> io::Result<Self> {
        use windows::Win32::Foundation::HANDLE;
        use windows::Win32::System::Memory::{
            MapViewOfFile, UnmapViewOfFile, FILE_MAP, FILE_MAP_READ, FILE_MAP_WRITE,
            MEMORY_MAPPED_VIEW_ADDRESS,
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

        // Validate the creator-written header before drain() can trust it. An
        // invalid capacity would cause a modulo-by-zero or an out-of-bounds
        // read once messages start flowing.
        let base = view.Value as *mut u8;
        let capacity = unsafe {
            let meta = &*(base.add(CACHE_LINE * 2) as *const MetaSlot);
            meta.capacity as usize
        };
        if capacity < 4 || HEADER_SIZE.saturating_add(capacity) > map_size {
            unsafe {
                let _ = UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS {
                    Value: base as *mut _,
                });
            }
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "ring buffer header capacity is invalid or exceeds mapped size",
            ));
        }

        Ok(Self {
            base,
            map_size,
            data_offset: HEADER_SIZE,
            mapping_handle,
            event_handle: None,
            owner: false,
        })
    }

    /// Raw mapping handle suitable for passing to a child process via stdio
    /// bootstrap. Only meaningful for readers created via `create_owner`.
    pub fn raw_handle(&self) -> RawHandle {
        self.mapping_handle.0 as RawHandle
    }

    /// Raw event handle suitable for passing to a child process via stdio
    /// bootstrap. Returns `None` for attach-created readers, which have no
    /// event of their own. Only `create_owner` readers return `Some`.
    pub fn raw_event_handle(&self) -> Option<RawHandle> {
        self.event_handle.map(|h| h.0 as RawHandle)
    }

    /// Blocks the current thread until the writer signals new data is
    /// available or the timeout elapses. Returns [`WaitOutcome::Signaled`] if
    /// the auto-reset event fired (and was automatically reset), or
    /// [`WaitOutcome::TimedOut`] if the timeout ran out first. In the timeout
    /// case the caller should still drain the ring as a belt-and-suspenders
    /// guard against lost signals.
    ///
    /// Only valid on readers created via `create_owner`. Attach-created
    /// readers return `ErrorKind::Unsupported`.
    pub fn wait_for_signal(&self, timeout_ms: u32) -> io::Result<WaitOutcome> {
        use windows::Win32::Foundation::{WAIT_FAILED, WAIT_OBJECT_0, WAIT_TIMEOUT};
        use windows::Win32::System::Threading::WaitForSingleObject;

        let Some(event) = self.event_handle else {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "wait_for_signal requires an owner-created reader",
            ));
        };

        let result = unsafe { WaitForSingleObject(event, timeout_ms) };
        match result {
            WAIT_OBJECT_0 => Ok(WaitOutcome::Signaled),
            WAIT_TIMEOUT => Ok(WaitOutcome::TimedOut),
            WAIT_FAILED => Err(io::Error::last_os_error()),
            other => Err(io::Error::other(format!(
                "WaitForSingleObject returned unexpected status {:#x}",
                other.0
            ))),
        }
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

    pub fn raw_event_handle(&self) -> Option<RawHandle> {
        None
    }

    pub fn wait_for_signal(&self, _timeout_ms: u32) -> io::Result<WaitOutcome> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "wait_for_signal not yet supported on this platform",
        ))
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
                if let Some(event) = self.event_handle {
                    let _ = CloseHandle(event);
                }
            }
        }
    }
}

#[cfg(windows)]
fn windows_err(err: windows::core::Error) -> io::Error {
    // Preserves the HRESULT message via the windows::core::Error's Display impl
    // rather than reducing it to a bare errno, which `from_raw_os_error(err.code().0)`
    // would do.
    io::Error::other(err)
}

/// Raw ring write path. Production writers live in the Go sidecar; this
/// method exists only for in-process test fixtures and the benchmark
/// harness, which need a Rust-side producer to exercise the drain hot
/// loop without spawning a child.
///
/// Gated behind `#[cfg(any(test, feature = "__bench"))]` so the writer
/// primitive never reaches a release build, and marked `#[doc(hidden)]`
/// so it is not part of the crate's public API surface.
#[cfg(all(windows, any(test, feature = "__bench")))]
impl RingBufReader {
    #[doc(hidden)]
    pub fn __bench_write(&self, payloads: &[&[u8]]) {
        let (write_slot, _, capacity) = self.header();
        let cap = capacity as usize;
        let data = self.data_ptr() as *mut u8;

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
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

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
        reader.__bench_write(&[b"hello world"]);

        let messages = reader.drain();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0], b"hello world");
    }

    #[test]
    fn write_and_read_multiple_messages() {
        let mut reader = RingBufReader::create_owner(4096).unwrap();
        reader.__bench_write(&[b"msg1", b"msg two", b"third message"]);

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

        reader.__bench_write(&[b"ABCDEFGH"]);

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
        owner.__bench_write(&[b"cross-view message"]);

        let messages = attached.drain();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0], b"cross-view message");
    }

    #[test]
    fn attach_rejects_zero_capacity_header() {
        // Corrupt the owner's MetaSlot.capacity to 0, then try to attach via
        // the raw handle. Attach must reject it instead of trusting the header,
        // otherwise drain() would hit a modulo-by-zero.
        let owner = RingBufReader::create_owner(4096).unwrap();
        unsafe {
            let meta = owner.base.add(CACHE_LINE * 2) as *mut MetaSlot;
            (*meta).capacity = 0;
        }
        let err = RingBufReader::attach(owner.raw_handle(), owner.map_size()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn attach_rejects_capacity_exceeding_map_size() {
        let owner = RingBufReader::create_owner(4096).unwrap();
        unsafe {
            let meta = owner.base.add(CACHE_LINE * 2) as *mut MetaSlot;
            (*meta).capacity = owner.map_size() as u64 + 1;
        }
        let err = RingBufReader::attach(owner.raw_handle(), owner.map_size()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn attach_rejects_capacity_below_minimum() {
        let owner = RingBufReader::create_owner(4096).unwrap();
        unsafe {
            let meta = owner.base.add(CACHE_LINE * 2) as *mut MetaSlot;
            (*meta).capacity = 2;
        }
        let err = RingBufReader::attach(owner.raw_handle(), owner.map_size()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn owner_exposes_event_handle_attach_does_not() {
        let owner = RingBufReader::create_owner(4096).unwrap();
        assert!(owner.raw_event_handle().is_some());
        assert_ne!(owner.raw_event_handle(), Some(0));

        let attached = RingBufReader::attach(owner.raw_handle(), owner.map_size()).unwrap();
        assert!(attached.raw_event_handle().is_none());
    }

    #[test]
    fn wait_for_signal_times_out_when_no_signal() {
        let owner = RingBufReader::create_owner(4096).unwrap();
        let start = std::time::Instant::now();
        let outcome = owner.wait_for_signal(20).unwrap();
        let elapsed = start.elapsed();
        assert_eq!(outcome, WaitOutcome::TimedOut);
        // Timeout should be roughly the requested 20ms, allow slack.
        assert!(elapsed >= std::time::Duration::from_millis(15));
        assert!(elapsed < std::time::Duration::from_millis(500));
    }

    #[test]
    fn wait_for_signal_wakes_on_set_event_from_another_thread() {
        use std::thread;
        use std::time::Duration;

        let owner = RingBufReader::create_owner(4096).unwrap();
        let event_raw = owner.raw_event_handle().expect("owner has event");

        // Spawn a helper that signals the event after a short delay. It uses
        // the raw handle integer, simulating what the Go sidecar does with the
        // inherited handle.
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(30));
            unsafe {
                use windows::Win32::Foundation::HANDLE;
                use windows::Win32::System::Threading::SetEvent;
                let _ = SetEvent(HANDLE(event_raw as *mut _));
            }
        });

        let start = std::time::Instant::now();
        let outcome = owner.wait_for_signal(1000).unwrap();
        let elapsed = start.elapsed();
        assert_eq!(outcome, WaitOutcome::Signaled);
        assert!(elapsed >= Duration::from_millis(25));
        assert!(elapsed < Duration::from_millis(500));
    }

    #[test]
    fn wait_for_signal_auto_resets_after_wake() {
        use std::time::Duration;

        let owner = RingBufReader::create_owner(4096).unwrap();
        let event_raw = owner.raw_event_handle().unwrap();

        unsafe {
            use windows::Win32::Foundation::HANDLE;
            use windows::Win32::System::Threading::SetEvent;
            let _ = SetEvent(HANDLE(event_raw as *mut _));
        }

        assert_eq!(owner.wait_for_signal(500).unwrap(), WaitOutcome::Signaled);
        let start = std::time::Instant::now();
        // Second wait on the already-consumed auto-reset event should time out.
        assert_eq!(owner.wait_for_signal(20).unwrap(), WaitOutcome::TimedOut);
        assert!(start.elapsed() >= Duration::from_millis(15));
    }

    #[test]
    fn wait_for_signal_rejects_attached_reader() {
        let owner = RingBufReader::create_owner(4096).unwrap();
        let attached = RingBufReader::attach(owner.raw_handle(), owner.map_size()).unwrap();
        let err = attached.wait_for_signal(10).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::Unsupported);
    }
}
