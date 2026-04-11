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

// createTestEvent creates an auto-reset unnamed Windows Event for Notify tests.
// The caller is responsible for closing the returned handle.
func createTestEvent(t *testing.T) windows.Handle {
	t.Helper()
	handle, err := windows.CreateEvent(nil, 0, 0, nil)
	if err != nil {
		t.Fatalf("CreateEvent: %v", err)
	}
	return handle
}

func TestNotifyRejectsZeroHandle(t *testing.T) {
	if err := Notify(0); err == nil {
		t.Fatal("expected error for zero event handle")
	}
}

func TestNotifyRejectsInvalidHandle(t *testing.T) {
	// An arbitrary non-zero value that is not a valid HANDLE. SetEvent returns
	// ERROR_INVALID_HANDLE which our wrapper surfaces as a wrapped error.
	if err := Notify(uintptr(0xDEADBEEF)); err == nil {
		t.Fatal("expected error for invalid event handle")
	}
}

func TestNotifySignalsEvent(t *testing.T) {
	handle := createTestEvent(t)
	defer func() { _ = windows.CloseHandle(handle) }()

	if err := Notify(uintptr(handle)); err != nil {
		t.Fatalf("Notify: %v", err)
	}

	// After Notify, a zero-timeout Wait should return WAIT_OBJECT_0 (auto-reset
	// consumes the signal). A second Wait should return WAIT_TIMEOUT.
	result, err := windows.WaitForSingleObject(handle, 0)
	if err != nil {
		t.Fatalf("WaitForSingleObject: %v", err)
	}
	if result != windows.WAIT_OBJECT_0 {
		t.Fatalf("expected WAIT_OBJECT_0 after Notify, got %#x", result)
	}

	result, err = windows.WaitForSingleObject(handle, 0)
	if err != nil {
		t.Fatalf("WaitForSingleObject second: %v", err)
	}
	if result != uint32(windows.WAIT_TIMEOUT) {
		t.Fatalf("expected WAIT_TIMEOUT after auto-reset, got %#x", result)
	}
}

func TestCloseEventHandleZeroIsNoOp(t *testing.T) {
	if err := CloseEventHandle(0); err != nil {
		t.Fatalf("expected zero handle to be no-op, got %v", err)
	}
}

func TestCloseEventHandleSuccess(t *testing.T) {
	handle := createTestEvent(t)
	if err := CloseEventHandle(uintptr(handle)); err != nil {
		t.Fatalf("CloseEventHandle: %v", err)
	}
}

func TestCloseEventHandleRejectsInvalid(t *testing.T) {
	// Closing a bogus handle should surface the underlying CloseHandle error.
	if err := CloseEventHandle(uintptr(0xDEADBEEF)); err == nil {
		t.Fatal("expected error closing invalid handle")
	}
}
