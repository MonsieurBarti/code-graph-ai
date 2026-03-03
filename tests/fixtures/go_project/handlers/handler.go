package handlers

import "fmt"

type Handler interface {
	Handle()
	Name() string
}

type Router struct {
	prefix string
}

type Server struct {
	Router
}

func (r *Router) Handle() {
	fmt.Println(r.prefix)
}

func (r *Router) Name() string {
	return r.prefix
}

func NewRouter(prefix string) *Router {
	return &Router{prefix: prefix}
}

type User struct {
	ID   int    `json:"id" gorm:"primaryKey"`
	Name string `json:"name"`
}

func Handle() {
	fmt.Println("handle")
}
