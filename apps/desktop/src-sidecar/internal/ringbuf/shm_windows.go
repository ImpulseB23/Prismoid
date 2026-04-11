//go:build windows

package ringbuf

import (
	"fmt"
	"unsafe"

	"golang.org/x/sys/windows"
)

const fileMapAllAccess = 0xF001F

// Attach maps a shared memory section that was created by the parent Rust host
// and inherited into this process via CreateProcess handle inheritance. The
// parent passes the raw handle value via the stdio bootstrap; this function
// takes ownership of that handle and closes it in the returned cleanup.
//
// The returned slice is backed by the mapped section. Writes are visible to
// any other process that has mapped the same section. The caller must call
// the returned cleanup exactly once when the section is no longer needed.
func Attach(handle uintptr, size int) ([]byte, func(), error) {
	if size <= 0 {
		return nil, nil, fmt.Errorf("invalid size %d", size)
	}
	if handle == 0 {
		return nil, nil, fmt.Errorf("invalid handle 0")
	}

	h := windows.Handle(handle)

	addr, err := windows.MapViewOfFile(h, fileMapAllAccess, 0, 0, uintptr(size))
	if err != nil {
		_ = windows.CloseHandle(h)
		return nil, nil, fmt.Errorf("MapViewOfFile: %w", err)
	}

	mem := unsafe.Slice((*byte)(unsafe.Pointer(addr)), size)

	cleanup := func() {
		_ = windows.UnmapViewOfFile(addr)
		_ = windows.CloseHandle(h)
	}

	return mem, cleanup, nil
}
