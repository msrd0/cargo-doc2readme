#!/bin/bash
set -euo pipefail

cargo=${CARGO:-cargo}
version="$($cargo fmt -- -V)"
case "$version" in
	*nightly*)
		# all good, no additional flags required
		;;
	*)
		# assume we're using some sort of rustup setup
		cargo="$cargo +nightly"
		;;
esac

return=0
while read file; do
	fail=no
	ok=yes

	# check if this is a test which is allowed to fail
	# also, ignore anything in the target folder
	case "$file" in
		*/target/*)
			continue
			;;
		*/fail/*)
			fail=yes
			;;
		*)
			;;
	esac

	echo -e "\e[1m ==> Formatting project $file ...\e[0m"

	# check that the project compiles (unless fail) without modifying the lock file
	cargo check --manifest-path "$file" --locked || ok=no
	echo "fail=$fail ok=$ok"
	if [ "$fail" == "yes" ] && [ "$ok" == "no" ] && ! cargo check --manifest-path "$file" &>/dev/null; then
		echo -e "\e[1;33m  -> Ignored\e[0m"
		continue
	fi

	# run rustfmt with the provided flags
	if [ "$ok" == "yes" ]; then
		$cargo fmt --manifest-path "$file" -- \
			--config-path "$(dirname "$0")/rustfmt.toml" "$@" \
			|| ok=no
	fi

	if [ "$ok" == "yes" ]; then
		echo -e "\e[1;32m  -> Success\e[0m"
	else
		echo -e "\e[1;31m  -> Failed\e[0m"
		return=1
	fi
done < <(find . -name 'Cargo.toml' -type f)

exit $return
