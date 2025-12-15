package main

import (
	"fmt"
	io "io"
)

type Reader interface {
	Read(p []byte) (n int, err error)
}

type Closer interface {
	Close() error
}

type ReadCloser interface {
	Reader
	Closer
}

type File struct {
	name string
	Reader // embedded interface
}

func (f *File) Read(p []byte) (int, error) {
	fmt.Println("reading")
	return 0, nil
}

func (f File) Close() error {
	return nil
}

func helper() {}

func (f *File) Use() {
	helper()
	f.Close()
	_ = io.EOF
}
