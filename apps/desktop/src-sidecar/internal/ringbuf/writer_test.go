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
