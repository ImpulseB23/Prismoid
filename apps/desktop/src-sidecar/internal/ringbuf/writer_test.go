package ringbuf

import (
	"encoding/binary"
	"sync/atomic"
	"testing"
	"unsafe"
)

func makeBuf(capacity int) []byte {
	mem := make([]byte, headerSize+capacity)
	*(*uint64)(unsafe.Pointer(&mem[cacheLine*2])) = uint64(capacity)
	return mem
}

func TestOpenValidatesSize(t *testing.T) {
	_, err := Open(make([]byte, 10))
	if err == nil {
		t.Fatal("expected error for small buffer")
	}
}

func TestOpenValidatesCapacity(t *testing.T) {
	mem := make([]byte, headerSize+64)
	_, err := Open(mem)
	if err == nil {
		t.Fatal("expected error for zero capacity")
	}
}

func TestWriteSingleMessage(t *testing.T) {
	mem := makeBuf(4096)
	w, err := Open(mem)
	if err != nil {
		t.Fatal(err)
	}

	ok := w.Write([]byte("hello"))
	if !ok {
		t.Fatal("write returned false")
	}

	writePos := atomic.LoadUint64(w.writeIndex())
	if writePos != 4+5 {
		t.Fatalf("expected write_index=9, got %d", writePos)
	}

	data := mem[headerSize:]
	msgLen := binary.BigEndian.Uint32(data[0:4])
	if msgLen != 5 {
		t.Fatalf("expected length=5, got %d", msgLen)
	}

	payload := string(data[4:9])
	if payload != "hello" {
		t.Fatalf("expected payload='hello', got '%s'", payload)
	}
}

func TestWriteMultipleMessages(t *testing.T) {
	mem := makeBuf(4096)
	w, err := Open(mem)
	if err != nil {
		t.Fatal(err)
	}

	messages := []string{"msg1", "msg two", "third message"}
	for _, msg := range messages {
		if !w.Write([]byte(msg)) {
			t.Fatalf("write failed for %q", msg)
		}
	}

	data := mem[headerSize:]
	offset := 0
	for i, expected := range messages {
		msgLen := int(binary.BigEndian.Uint32(data[offset : offset+4]))
		offset += 4
		got := string(data[offset : offset+msgLen])
		if got != expected {
			t.Fatalf("message %d: expected %q, got %q", i, expected, got)
		}
		offset += msgLen
	}
}

func TestWriteFullBuffer(t *testing.T) {
	mem := makeBuf(32)
	w, err := Open(mem)
	if err != nil {
		t.Fatal(err)
	}

	ok := w.Write(make([]byte, 20))
	if !ok {
		t.Fatal("first write should succeed")
	}

	ok = w.Write(make([]byte, 20))
	if ok {
		t.Fatal("second write should fail (buffer full)")
	}
}

func TestOpenValidatesMemSizeForCapacity(t *testing.T) {
	mem := make([]byte, headerSize+64)
	// claim capacity of 1024 but buffer only has 64 bytes of data space
	*(*uint64)(unsafe.Pointer(&mem[cacheLine*2])) = 1024
	_, err := Open(mem)
	if err == nil {
		t.Fatal("expected error when mem is too small for declared capacity")
	}
}

func TestWriteWrapsAround(t *testing.T) {
	mem := makeBuf(32)
	w, err := Open(mem)
	if err != nil {
		t.Fatal(err)
	}

	// write 20 bytes (4-byte header + 16-byte payload)
	if !w.Write(make([]byte, 16)) {
		t.Fatal("first write should succeed")
	}

	// simulate reader consuming those 20 bytes
	atomic.StoreUint64(w.readIndex(), 20)

	// write 12-byte payload (framed: 4 + 12 = 16 bytes)
	// write_pos=20, cap=32, offset=20%32=20, firstChunk=12
	// length header (4 bytes at offset 20): fits without wrapping
	// payload (12 bytes at offset 24): firstChunk=8 < 12, wraps around
	payload := []byte("ABCDEFGHIJKL")
	if !w.Write(payload) {
		t.Fatal("wrapped write should succeed")
	}

	data := w.data()

	msgLen := binary.BigEndian.Uint32(data[20:24])
	if msgLen != 12 {
		t.Fatalf("expected length=12, got %d", msgLen)
	}

	// payload: 8 bytes at [24..32), then 4 bytes at [0..4)
	var got [12]byte
	copy(got[:8], data[24:32])
	copy(got[8:], data[0:4])
	if string(got[:]) != "ABCDEFGHIJKL" {
		t.Fatalf("expected 'ABCDEFGHIJKL', got %q", string(got[:]))
	}
}
