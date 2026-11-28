#!/usr/bin/env bash
# Writes every vector file in this directory again with the Go toolchain on the PATH. Run from anywhere.
#
# Three kinds of file are written only by Go 1.27.0, because they hold what that release's JSON encoder writes: the two
# JSON files, which define a format, and the record scanner's rows, whose inputs are built with the encoder. Any other
# release leaves them alone.
set -eu
set -o pipefail
here=$(cd "$(dirname "$0")" && pwd -P)
root=$(cd "$here/../.." && pwd -P)
cd "$root"
new=$(mktemp -d)
trap 'rm -rf "$new" "$root/zz_rust_vectors_test.go" "$root/cmd/dost-conform/zz_rust_vectors_test.go"' EXIT
version=$(go env GOVERSION)

go run rust/testdata/seam_vectors.go > "$new/seam-go.txt"
go run rust/testdata/wire_vectors.go > "$new/wire-go.txt"
go run rust/testdata/wire_more_vectors.go > "$new/wire-more-go.txt"
go run rust/testdata/gojson_vectors.go > "$new/gojson-strings.txt"
go run rust/testdata/gojson_reader_vectors.go > "$new/gojson-reader.txt"
# The format vectors come from unexported functions, and the driver's from its unexported request type, so each writer
# is compiled inside its package for one run.
cp "$here/internal_vectors_test.go.txt" "$root/zz_rust_vectors_test.go"
DOST_RUST_VECTORS="$new" go test -count=1 -run '^TestWriteRustVectors$' -v . | grep -E 'rows|ok|FAIL'
cp "$here/driver_vectors_test.go.txt" "$root/cmd/dost-conform/zz_rust_vectors_test.go"
DOST_RUST_VECTORS="$new" go test -count=1 -run '^TestWriteRustDriverVectors$' -v ./cmd/dost-conform | grep -E 'rows|ok|FAIL'

for file in "$new"/*.txt; do
 name=$(basename "$file")
 case "$name" in
 gojson-strings.txt | gojson-reader.txt | recordscan-go.txt)
  if [ "$version" != go1.27.0 ]; then
   echo "left $name alone: this is $version, it needs go1.27.0"
   continue
  fi
  ;;
 esac
 cp "$file" "$here/$name"
done
