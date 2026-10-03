#!/bin/bash
# Build an agent branch on GitHub CI instead of the laptop.
#
#   .claude/ci-branch.sh push <local-branch> <name>   # push to GitHub as agent/<name>
#   .claude/ci-branch.sh wait <name>                   # wait for CI, print results and failures
#   .claude/ci-branch.sh delete <name>                 # remove agent/<name> after merging
#
# Only the laptop has GitHub credentials, so the branch goes there first.
# agent/* branches never produce installs; only dev does.
set -euo pipefail
LAPTOP=jacob@192.168.18.52
REPO=millionrust/lulo
cmd=${1:?push|wait|delete}
case "$cmd" in
  push)
    branch=${2:?local branch}; name=${3:?name}
    git push -q -f "ssh://$LAPTOP/home/jacob/rmac" "$branch:refs/heads/claude/ci-$name"
    ssh -o BatchMode=yes "$LAPTOP" "git -C ~/rmac push -q -f https://github.com/$REPO.git claude/ci-$name:refs/heads/agent/$name && git -C ~/rmac rev-parse claude/ci-$name"
    ;;
  wait)
    name=${2:?name}
    ssh -o BatchMode=yes "$LAPTOP" "
      sha=\$(git -C ~/rmac rev-parse claude/ci-$name)
      until s=\$(gh run list -R $REPO --commit \$sha --json status,conclusion,name,databaseId -q '.[]|\"\(.name) \(.status) \(.conclusion) \(.databaseId)\"'); [ -n \"\$s\" ] && ! echo \"\$s\" | grep -qv ' completed '; do sleep 60; done
      echo \"\$s\"
      echo \"\$s\" | awk '/ failure /{print \$NF}' | while read id; do
        gh run view -R $REPO \$id --json jobs -q '.jobs[]|\"  job: \(.name): \(.conclusion)\"'
        gh run view -R $REPO \$id --log-failed 2>/dev/null | sed 's/\x1b\[[0-9;]*m//g' \
          | grep -E 'error(\[E[0-9]+\])?:|panicked at|test result: FAILED|AssertionError|^[^ ]+ +[^ ]+ +[^ ]+ +FAIL ' \
          | grep -v 'Process completed\|no runner error' | sort -u | head -25 | cut -c1-240
      done"
    # The summary starts non-blocking; print its artifact even if the workflow succeeds.
    ssh -o BatchMode=yes "$LAPTOP" "
      sha=\$(git -C ~/rmac rev-parse claude/ci-$name)
      id=\$(gh run list -R $REPO --commit \$sha --json name,databaseId -q '.[]|select(.name==\"Lulo runtime\")|.databaseId' | head -1)
      if [ -n \"\$id\" ]; then
        dir=\$(mktemp -d)
        if gh run download -R $REPO \$id --name lulo-runtime-summary --dir \"\$dir\" >/dev/null 2>&1; then
          cat \"\$dir/runtime-summary.md\"
        else
          echo 'Lulo runtime summary artifact unavailable'
        fi
        rm -rf \"\$dir\"
      fi"
    ;;
  delete)
    name=${2:?name}
    ssh -o BatchMode=yes "$LAPTOP" "git -C ~/rmac push -q https://github.com/$REPO.git :refs/heads/agent/$name; git -C ~/rmac branch -q -D claude/ci-$name 2>/dev/null || true"
    ;;
  *) echo "usage: $0 push|wait|delete" >&2; exit 2;;
esac
