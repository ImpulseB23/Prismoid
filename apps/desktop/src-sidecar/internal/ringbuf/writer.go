package ringbuf

import (
	"encoding/binary"
	"fmt"
	"sync/atomic"
	"unsafe"
)

const (
	cacheLine  = 64
	headerSize = cacheLine * 3
)

type Writer struct {
	mem      []byte
	capacity uint64
}

func Open(mem []byte) (*Writer, error) {
	if len(mem) < headerSize {
		return nil, fmt.Errorf("shared memory too small: %d bytes, need at least %d", len(mem), headerSize)
	}

	capacity := *(*uint64)(unsafe.Pointer(&mem[cacheLine*2]))
	if capacity == 0 {
		return nil, fmt.Errorf("ring buffer not initialized (capacity is 0)")
	}

	if uint64(len(mem)) < uint64(headerSize)+capacity {
		return nil, fmt.Errorf("shared memory size %d too small for capacity %d", len(mem), capacity)
	}

	return &Writer{mem: mem, capacity: capacity}, nil
}

func (w *Writer) writeIndex() *uint64 {
	return (*uint64)(unsafe.Pointer(&w.mem[0]))
}

func (w *Writer) readIndex() *uint64 {
	return (*uint64)(unsafe.Pointer(&w.mem[cacheLine]))
}

func (w *Writer) data() []byte {
	return w.mem[headerSize:]
}

func (w *Writer) Write(payload []byte) bool {
	if len(payload) > 1<<30 {
		return false
	}
	msgSize := uint64(4 + len(payload))
	cap := w.capacity

	writePos := atomic.LoadUint64(w.writeIndex())
	readPos := atomic.LoadUint64(w.readIndex())

	if writePos-readPos+msgSize > cap {
		return false
	}

	data := w.data()

	var lenBuf [4]byte
	binary.BigEndian.PutUint32(lenBuf[:], uint32(len(payload)))
	w.writeWrapped(data, writePos, cap, lenBuf[:])
	w.writeWrapped(data, writePos+4, cap, payload)

	atomic.StoreUint64(w.writeIndex(), writePos+msgSize)

	return true
}

func (w *Writer) writeWrapped(data []byte, pos, cap uint64, src []byte) {
	offset := pos % cap
	firstChunk := cap - offset

	if firstChunk >= uint64(len(src)) {
		copy(data[offset:], src)
	} else {
		copy(data[offset:], src[:firstChunk])
		copy(data[:], src[firstChunk:])
	}
}
