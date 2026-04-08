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

func TestOpenSharedNonexistent(t *testing.T) {
	_, _, err := OpenShared("nonexistent_prismoid_test_shm", 4096)
	if err == nil {
		t.Fatal("expected error for nonexistent shared memory")
	}
}

func TestOpenSharedRoundTrip(t *testing.T) {
	name := "prismoid_test_shm_roundtrip"
	size := 4096

	handle := createTestMapping(t, name, uint32(size))
	defer func() { _ = windows.CloseHandle(handle) }()

	mem, cleanup, err := OpenShared(name, size)
	if err != nil {
		t.Fatalf("OpenShared: %v", err)
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
