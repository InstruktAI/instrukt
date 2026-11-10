#!/bin/sh
# Record a new nightly as a dependency head, then explicitly dispatch its check.
set -eu
branch=upstream/t3-nightly
pin=crates/instrukt-runner/t3.version
default_branch=${DEFAULT_BRANCH:?default branch is required}
remote_record=$(git ls-remote --tags --refs --sort=-version:refname https://github.com/pingdotgg/t3code.git 'v*-nightly.*' | awk 'NR == 1 { print $1, $2 }')
upstream_sha=${remote_record%% *}
remote_tag=${remote_record#* }
[ -n "$remote_tag" ] || { echo 'No nightly tag found' >&2; exit 1; }
version=${remote_tag#refs/tags/v}
case "$version" in *[!a-zA-Z0-9.-]*|'') echo 'Invalid nightly version' >&2; exit 1;; esac
old_head=$(git ls-remote --heads origin "refs/heads/$branch" | awk '{ print $1 }')
if [ -n "$old_head" ]; then
    git fetch --no-tags origin "refs/heads/$branch"
    recorded=$(git show "FETCH_HEAD:$pin")
    if [ "$recorded" = "$version" ]; then
        echo 'The dependency branch already records this nightly.'
        exit 0
    fi
fi
git fetch --no-tags origin "refs/heads/$default_branch"
git switch --detach FETCH_HEAD
printf '%s\n' "$version" > "$pin"
git config user.name 'github-actions[bot]'
git config user.email '41898282+github-actions[bot]@users.noreply.github.com'
git add "$pin"
message=$(printf 'chore(deps): check T3 %s' "$version")
git commit -m "$message"
git push --force-with-lease="refs/heads/$branch:$old_head" origin "HEAD:refs/heads/$branch"
# GITHUB_TOKEN pushes do not start push workflows; dispatch is the exception.
gh workflow run t3-contract.yml --ref "$branch" -f "upstream_sha=$upstream_sha"
