//go:build linux

package ringbuf

import (
	"fmt"
	"os"
	"syscall"
)

// OpenShared opens a named shared memory region created by the Rust host.
func OpenShared(name string, size int) ([]byte, func(), error) {
	path := "/dev/shm/" + name

	f, err := os.OpenFile(path, os.O_RDWR, 0)
	if err != nil {
		return nil, nil, fmt.Errorf("open shm %s: %w", path, err)
	}

	mem, err := syscall.Mmap(int(f.Fd()), 0, size, syscall.PROT_READ|syscall.PROT_WRITE, syscall.MAP_SHARED)
	if err != nil {
		f.Close()
		return nil, nil, fmt.Errorf("mmap: %w", err)
	}

	cleanup := func() {
		syscall.Munmap(mem)
		f.Close()
	}

	return mem, cleanup, nil
}
