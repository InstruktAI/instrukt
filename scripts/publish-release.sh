#!/bin/sh
# Publish the complete tagged asset set only after version and presence checks.
set -eu
tag=${GITHUB_REF_NAME:?GITHUB_REF_NAME is required}
repository=${GITHUB_REPOSITORY:?GITHUB_REPOSITORY is required}
script_dir=$(CDPATH='' cd -- "$(dirname -- "$0")" && pwd)
"$script_dir/check-release-version.sh" "$tag"
for target in aarch64-apple-darwin x86_64-unknown-linux-gnu aarch64-unknown-linux-gnu; do
    asset=instrukt-$tag-$target.tar.gz
    if [ ! -f "$asset" ]; then printf 'missing release asset: %s\n' "$asset" >&2; exit 1; fi
done
sha256sum instrukt-*.tar.gz > SHA256SUMS
gh release create "$tag" instrukt-*.tar.gz SHA256SUMS "$script_dir/install.sh" --repo "$repository" --generate-notes --draft --verify-tag
gh release edit "$tag" --repo "$repository" --draft=false --latest
