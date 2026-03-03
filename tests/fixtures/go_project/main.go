package main

import (
	"fmt"
	"github.com/test/goproject/handlers"
)

//go:generate stringer -type=Color

type Color int

const (
	Red Color = iota
	Green
	Blue
)

var AppName = "TestApp"

func main() {
	fmt.Println("Hello")
	handlers.Handle()
}

func init() {
	fmt.Println("init")
}
