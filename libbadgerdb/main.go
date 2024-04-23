package main

import "C"
import (
	badger "github.com/dgraph-io/badger/v4"
)

// go build -o libbadgerdb.so -buildmode=c-shared badgerdb.go

var dbMap map[string]*badger.DB

//export OpenDB
func OpenDB(path *C.char) *C.char {
	options := badger.DefaultOptions(C.GoString(path))

	db, err := badger.Open(options)
	if err != nil {
		return C.CString(err.Error())
	}
	dbMap[C.GoString(path)] = db
	return path
}

func main() {}
