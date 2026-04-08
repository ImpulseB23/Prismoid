use std::sync::atomic::{AtomicU64, Ordering};

const CACHE_LINE: usize = 64;
const HEADER_SIZE: usize = CACHE_LINE * 3;

pub const DEFAULT_CAPACITY: usize = 4 * 1024 * 1024; // 4MB
const SHM_NAME: &str = "prismoid_ringbuf";

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

pub struct RingBufReader {
    shmem: shared_memory::Shmem,
    data_offset: usize,
}

impl RingBufReader {
    pub fn create(capacity: usize) -> Result<Self, shared_memory::ShmemError> {
        Self::create_named(SHM_NAME, capacity)
    }

    pub fn create_named(name: &str, capacity: usize) -> Result<Self, shared_memory::ShmemError> {
        assert!(
            capacity >= 4,
            "ring buffer capacity must be at least 4 bytes"
        );
        let total = HEADER_SIZE
            .checked_add(capacity)
            .expect("ring buffer total size overflowed usize");

        let shmem = shared_memory::ShmemConf::new()
            .os_id(name)
            .size(total)
            .create()?;

        let ptr = shmem.as_ptr();

        unsafe {
            std::ptr::write_bytes(ptr, 0, total);

            let meta_slot = &mut *(ptr.add(CACHE_LINE * 2) as *mut MetaSlot);
            meta_slot.capacity = capacity as u64;

            let write_slot = &*(ptr as *const WriteSlot);
            let read_slot = &*(ptr.add(CACHE_LINE) as *const ReadSlot);
            write_slot.index.store(0, Ordering::Release);
            read_slot.index.store(0, Ordering::Release);
        }

        Ok(Self {
            shmem,
            data_offset: HEADER_SIZE,
        })
    }

    pub fn os_id(&self) -> &str {
        self.shmem.get_os_id()
    }

    fn header(&self) -> (*const WriteSlot, *const ReadSlot, u64) {
        let ptr = self.shmem.as_ptr();
        unsafe {
            let write_slot = &*(ptr as *const WriteSlot);
            let read_slot = &*(ptr.add(CACHE_LINE) as *const ReadSlot);
            let meta = &*(ptr.add(CACHE_LINE * 2) as *const MetaSlot);
            (write_slot, read_slot, meta.capacity)
        }
    }

    fn data_ptr(&self) -> *const u8 {
        unsafe { self.shmem.as_ptr().add(self.data_offset) }
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
        self.read_wrapped(data, pos, cap, &mut buf);
        u32::from_be_bytes(buf)
    }

    unsafe fn read_wrapped(&self, data: *const u8, pos: usize, cap: usize, out: &mut [u8]) {
        let offset = pos % cap;
        let first_chunk = cap - offset;

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

#[cfg(test)]
mod tests {
    use super::*;

    fn write_to_buf(reader: &RingBufReader, payloads: &[&[u8]]) {
        let (write_slot, _, capacity) = reader.header();
        let cap = capacity as usize;
        let data = reader.shmem.as_ptr() as usize + HEADER_SIZE;

        unsafe {
            let mut write_pos = (*write_slot).index.load(Ordering::Relaxed) as usize;
            for payload in payloads {
                let len_bytes = (payload.len() as u32).to_be_bytes();
                let offset = write_pos % cap;
                let first_chunk = cap - offset;

                // write length (4 bytes), handling wrap
                if first_chunk >= 4 {
                    std::ptr::copy_nonoverlapping(
                        len_bytes.as_ptr(),
                        (data + offset) as *mut u8,
                        4,
                    );
                } else {
                    std::ptr::copy_nonoverlapping(
                        len_bytes.as_ptr(),
                        (data + offset) as *mut u8,
                        first_chunk,
                    );
                    std::ptr::copy_nonoverlapping(
                        len_bytes.as_ptr().add(first_chunk),
                        data as *mut u8,
                        4 - first_chunk,
                    );
                }

                // write payload, handling wrap
                let pay_offset = (write_pos + 4) % cap;
                let pay_first = cap - pay_offset;
                if pay_first >= payload.len() {
                    std::ptr::copy_nonoverlapping(
                        payload.as_ptr(),
                        (data + pay_offset) as *mut u8,
                        payload.len(),
                    );
                } else {
                    std::ptr::copy_nonoverlapping(
                        payload.as_ptr(),
                        (data + pay_offset) as *mut u8,
                        pay_first,
                    );
                    std::ptr::copy_nonoverlapping(
                        payload.as_ptr().add(pay_first),
                        data as *mut u8,
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
    fn create_and_drain_empty() {
        let mut reader = RingBufReader::create_named("test_drain_empty_2", 4096).unwrap();
        let messages = reader.drain();
        assert!(messages.is_empty());
    }

    #[test]
    fn create_default_and_os_id() {
        let reader = RingBufReader::create(4096).unwrap();
        assert!(reader.os_id().contains(SHM_NAME));
    }

    #[test]
    fn write_and_read_single_message() {
        let mut reader = RingBufReader::create_named("test_single_msg_2", 4096).unwrap();
        write_to_buf(&reader, &[b"hello world"]);

        let messages = reader.drain();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0], b"hello world");
    }

    #[test]
    fn write_and_read_multiple_messages() {
        let mut reader = RingBufReader::create_named("test_multi_msg_2", 4096).unwrap();
        write_to_buf(&reader, &[b"msg1", b"msg two", b"third message"]);

        let messages = reader.drain();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0], b"msg1");
        assert_eq!(messages[1], b"msg two");
        assert_eq!(messages[2], b"third message");
    }

    #[test]
    fn drain_stops_on_corrupt_length() {
        let mut reader = RingBufReader::create_named("test_corrupt_len", 256).unwrap();
        let (write_slot, _, _) = reader.header();
        let data = reader.shmem.as_ptr() as usize + HEADER_SIZE;

        let bad_len: u32 = 257;
        let len_bytes = bad_len.to_be_bytes();

        unsafe {
            std::ptr::copy_nonoverlapping(len_bytes.as_ptr(), data as *mut u8, 4);
            (*write_slot)
                .index
                .store(4 + bad_len as u64, Ordering::Release);
        }

        let messages = reader.drain();
        assert!(messages.is_empty());
    }

    #[test]
    fn drain_stops_on_partial_message() {
        let mut reader = RingBufReader::create_named("test_partial_msg", 4096).unwrap();
        let (write_slot, _, _) = reader.header();
        let data = reader.shmem.as_ptr() as usize + HEADER_SIZE;

        let len_bytes = (100u32).to_be_bytes();

        unsafe {
            std::ptr::copy_nonoverlapping(len_bytes.as_ptr(), data as *mut u8, 4);
            (*write_slot).index.store(54, Ordering::Release);
        }

        let messages = reader.drain();
        assert!(messages.is_empty());
    }

    #[test]
    fn read_wraps_around_boundary() {
        let mut reader = RingBufReader::create_named("test_wrap_read", 32).unwrap();
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
}
