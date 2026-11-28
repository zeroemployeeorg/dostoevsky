#!/bin/sh
set -eu
version=$1
# Release builds are made with one Go release: the one whose output defines the stored and sent bytes.
want=go1.27.0
got=$("${GO:-go}" env GOVERSION)
[ "$got" = "$want" ] || { echo "release needs $want, found $got" >&2; exit 1; }
root=$(pwd -P)
name=dostoevsky-$version
mkdir -p dist
stage=$(mktemp -d "$root/dist/.release.XXXXXX")
trap 'rm -rf -- "$stage"' EXIT
if [ "${2:-build}" = build ]; then
 mkdir "$stage/$name"
 cp *.go go.mod LICENSE README.md CHANGELOG.md VERSION Makefile "$stage/$name/"
 cp -R examples scripts docs sketch benchmarks cmd clienthttp clients wire wireserver workflow internal rust "$stage/$name/"
 (cd "$stage" && tar --sort=name --mtime=@0 --owner=0 --group=0 --numeric-owner -cf "$name.tar" "$name" && gzip -n "$name.tar" && sha256sum "$name.tar.gz" > "$name.sha256")
 location=$stage
else
 location=$root/dist
fi
(cd "$location" && sha256sum -c "$name.sha256")
mkdir "$stage/check"
tar -xzf "$location/$name.tar.gz" -C "$stage/check"
(cd "$stage/check/$name" && GOPROXY=off CGO_ENABLED=0 "$GO" test ./...)
if [ "$location" = "$stage" ]; then
 mv "$stage/$name.tar.gz" "$stage/$name.sha256" dist/
fi
