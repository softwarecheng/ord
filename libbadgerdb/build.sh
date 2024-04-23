#!/bin/bash

OS_TYPE=$(uname -s)
LIB_NAME="libbadgerdb"
case $OS_TYPE in
    Darwin)
        echo "Building shared library for macOS..."
        go build -o "target/${LIB_NAME}.dylib" -buildmode=c-shared
    ;;
    Linux)
        echo "Building shared library for Linux..."
        go build -o "target/${LIB_NAME}.so" -buildmode=c-shared
    ;;
    *)
        echo "Unsupported OS type: $OS_TYPE"
        exit 1
    ;;
esac

echo "Shared library has been built and copied to $TARGET_DIR"