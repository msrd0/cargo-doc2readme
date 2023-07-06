#!/bin/bash
set -euo pipefail

cd "$(dirname "$0")"

function get_version() {
	cargo metadata --format-version=1 --no-deps --locked \
		| jq -r '.packages[].version'
}

if [ -n "$(git status --untracked-files=no --porcelain)" ]; then
	echo "Your working directory is not clean"
	exit 1
fi

echo   "Current Version:" $(get_version)
printf "New Version:     "
read version0
version="$(echo "$version0" | tr -d '[:blank:]')"
sed -E -e '/^\[package\]$/,/^\[/ {/version =/ {s/"[^"]*"/"'"$version"'"/; :a;n;ba}}' \
	-i Cargo.toml

if [ "$(get_version)" != "$version" ]; then
	echo "Failed to set version"
	exit 1
fi

cargo update -p cargo-doc2readme
git commit Cargo.toml Cargo.lock -m "Release cargo-doc2readme $version"
git push
git tag -s -a -m "Version $version" "$version"
git push --tags
cargo publish --locked
