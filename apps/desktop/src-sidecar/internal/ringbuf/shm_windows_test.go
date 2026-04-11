//go:build windows

package ringbuf

import (
	"testing"

	"golang.org/x/sys/windows"
)

func createTestMapping(t *testing.T, name string, size uint32) windows.Handle {
	t.Helper()
	namePtr, err := windows.UTF16PtrFromString(name)
	if err != nil {
		t.Fatal(err)
	}
	handle, err := windows.CreateFileMapping(
		windows.InvalidHandle,
		nil,
		windows.PAGE_READWRITE,
		0,
		size,
		namePtr,
	)
	if err != nil {
		t.Fatalf("CreateFileMapping: %v", err)
	}
	return handle
}

func TestAttachRejectsZeroHandle(t *testing.T) {
	_, _, err := Attach(0, 4096)
	if err == nil {
		t.Fatal("expected error for zero handle")
	}
}

func TestAttachRejectsZeroSize(t *testing.T) {
	handle := createTestMapping(t, "prismoid_test_attach_zero_size", 4096)
	defer func() { _ = windows.CloseHandle(handle) }()

	_, _, err := Attach(uintptr(handle), 0)
	if err == nil {
		t.Fatal("expected error for zero size")
	}
}

func TestAttachRoundTrip(t *testing.T) {
	const size = 4096

	// Attach takes ownership of the handle; do NOT also defer CloseHandle here.
	handle := createTestMapping(t, "prismoid_test_attach_roundtrip", size)

	mem, cleanup, err := Attach(uintptr(handle), size)
	if err != nil {
		_ = windows.CloseHandle(handle)
		t.Fatalf("Attach: %v", err)
	}
	defer cleanup()

	if len(mem) != size {
		t.Fatalf("expected len=%d, got %d", size, len(mem))
	}

	mem[0] = 0xAB
	if mem[0] != 0xAB {
		t.Fatal("shared memory read/write failed")
	}
}
