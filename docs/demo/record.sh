#!/bin/sh
# Records docs/demo.gif from real screens of an isolated orcux: a home, a repo and servers of its own, pretend agents
# (docs/demo/agent.pl) and an outer tmux that plays the person's terminal. Nothing of the person's is touched.
#   docs/demo/record.sh          record docs/demo.gif (cargo build --release first)
#   docs/demo/record.sh setup    only set the scene up, to look at it: tmux -S ~/.cache/orcux-demo/outer attach
#   docs/demo/record.sh clean    stop and delete everything it started
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(cd "$here/../.." && pwd)
bin=${ORCUX_BIN:-$root/target/release/orcux}
# never the person's servers: no TMUX, and a folder of sockets of its own
unset TMUX TMUX_PANE ORCUX_MUX ORCUX_ID ORCUX_SLOT ORCUX_PORT GH_TOKEN GITHUB_TOKEN COMPOSE_PROJECT_NAME
export TMUX_TMPDIR="$HOME/.cache/orcux-demo"
real_home=$HOME
outer="$TMUX_TMPDIR/outer"
COLS=${COLS:-140}                                    # the rail of pull requests shows from 140
ROWS=${ROWS:-34}

clean() {
    [ -d "$TMUX_TMPDIR" ] || return 0
    tmux -S "$outer" kill-server 2>/dev/null || true
    if [ -f "$TMUX_TMPDIR/home" ]; then
        h=$(cat "$TMUX_TMPDIR/home")
        HOME=$h "$bin" tmux kill-server 2>/dev/null || true
    fi
    pkill -f "mux-server $TMUX_TMPDIR/" 2>/dev/null || true
    sleep 1
    case ${h:-} in */orcux-demo.*) rm -rf "$h" 2>/dev/null || { sleep 1; rm -rf "$h"; } ;; esac
    rm -rf "$TMUX_TMPDIR"
}

if [ "${1:-}" = clean ]; then clean; exit 0; fi
[ -x "$bin" ] || { echo "no $bin: cargo build --release first" >&2; exit 1; }
clean
mkdir -p "$TMUX_TMPDIR"
export HOME=$(mktemp -d "${TMPDIR:-/tmp}/orcux-demo.XXXXXX")
echo "$HOME" > "$TMUX_TMPDIR/home"
if [ "${1:-}" != setup ]; then trap clean EXIT; trap 'exit 1' INT TERM; fi

# a neutral prompt and names: nothing of the computer or the person shows
mkdir -p "$HOME/bin" "$HOME/.demo"
ln -s "$bin" "$HOME/bin/orcux"
cp "$here/gh.sh" "$HOME/bin/gh"                        # made-up pull requests, and no GitHub at all
printf '#!/bin/sh\nexit 0\n' > "$HOME/bin/osascript"     # no notifications
chmod +x "$HOME/bin/gh" "$HOME/bin/osascript"
cp /bin/bash "$HOME/bin/agent"                          # so that the tabs say "agent"
if command -v codesign >/dev/null; then codesign -s - -f "$HOME/bin/agent" 2>/dev/null; fi   # a copy must be signed again
printf '#!/bin/sh\nperl "$HOME/.demo/agent.pl" "$1"\n' > "$HOME/.demo/run"
cp "$here/agent.pl" "$HOME/.demo/agent.pl"
printf "PS1='%%F{8}\$%%f '\nunsetopt PROMPT_SP\n" > "$HOME/.zshrc"
export PATH="$HOME/bin:$PATH"
export SHELL=$(command -v zsh || command -v bash) PS1='$ ' LANG=en_US.UTF-8 TERM=xterm-256color
export ORCUX_ENGINE=own ORCUX_LAUNCHD=0 ORCUX_UI=window ORCUX_LANG=en
export GIT_AUTHOR_DATE='2026-11-26T10:00:00' GIT_COMMITTER_DATE='2026-11-26T10:00:00'
git config --global user.name demo
git config --global user.email demo@example.com
git config --global init.defaultBranch main
git config --global advice.detachedHead false

# two repos, each with an origin of its own: a local bare repo, under "github/" so that orcux asks gh for its PRs
repo() {
    git init -q --bare "$HOME/github/demo/$1.git"
    git init -q "$HOME/code/$1"
    cp -R "$here/$1/." "$HOME/code/$1/"
    (cd "$HOME/code/$1" && git add -A && git commit -qm "First version" && git remote add origin "$HOME/github/demo/$1.git" \
        && git push -q origin main && git remote set-head origin main)
}
repo shop
repo api
orcux notify off
orcux icons plain                                     # the outer tmux drops Nerd Font glyphs
cd "$HOME/code/shop"
for w in checkout login-fix search; do orcux new "$w" >/dev/null; done
cd "$HOME/code/api"
orcux new rate-limit >/dev/null

# what the agent of checkout did: a commit, and a change left for review
wt="$HOME/orcux/shop/checkout"
cp "$here/change/Summary.tsx" "$wt/src/checkout/Summary.tsx"
(cd "$wt" && git commit -qam "Show the cart total at checkout")
cp "$here/change/total.ts" "$wt/src/cart/total.ts"

agent() { orcux send "$1" "agent ~/.demo/run $2"; }
agent checkout done
agent login-fix ask
agent search work
agent rate-limit work
orcux tab new search >/dev/null
orcux send search:2 'echo "dev server on port $ORCUX_PORT"'
orcux show search

# the person's terminal
tmux -S "$outer" -f /dev/null new-session -d -s demo -x "$COLS" -y "$ROWS" "env -u TMUX -u TMUX_PANE orcux; sleep 600"
tmux -S "$outer" set -g status off
[ "${1:-}" = setup ] && { echo "scene up: tmux -S $outer attach"; exit 0; }

# ---------- the film: keys as a person would press them, and a capture of the screen after each ----------
frames="$TMUX_TMPDIR/frames"
mkdir -p "$frames"
n=0
press() { tmux -S "$outer" send-keys -t demo "$@"; }
screen() { tmux -S "$outer" capture-pane -p -t demo; }
# until the screen shows a text
until_on() {
    i=0
    until screen | grep -qF -- "$1"; do
        i=$((i + 1)); [ $i -lt 150 ] || { echo "never on screen: $1" >&2; screen >&2; exit 1; }
        sleep 0.1
    done
}
# one frame, shown for some milliseconds, with a caption "keys|what they do"
shot() {
    n=$((n + 1)); f=$(printf '%03d.ansi' $n)
    tmux -S "$outer" capture-pane -e -N -p -t demo > "$frames/$f"      # -N: the colored cells at the right end too
    printf '%s	%s	%s
' "$f" "$1" "${2:-}" >> "$frames/list.tsv"
}
# several frames of what moves, in real time
film() {
    i=0
    while [ $i -lt "$1" ]; do shot "$2" "${3:-}"; sleep "$(awk "BEGIN { print $2 / 1000 }")"; i=$((i + 1)); done
}
esc=$(printf '\033')
click() { press -l "$esc[<0;$(($1 + 1));$(($2 + 1))M"; press -l "$esc[<0;$(($1 + 1));$(($2 + 1))m"; }

until_on '1 agent waits for you'
until_on 'Working'
until_on '2 zsh ─'      # the shell of the second tab printed during the setup: until it is still
sleep 1
film 14 150 '|● works   ? asks you   ■ done'
press M-r; until_on 'Waiting for you'; sleep 0.3
shot 2200 'Opt r|what waits for you, oldest first'
press Enter; until_on 'Run this command?'; sleep 0.5
shot 1700 '⏎|straight to the agent that asks'
press 1; until_on '12 passed'; until_on '┤ 1 ● agent'; sleep 0.3
film 10 150 '1|answered: it works again'
press M-r; until_on 'Waiting for you'; sleep 0.3
shot 1300 'Opt r|the next one: done, and your PRs'
press Enter; until_on 'waits for your review'; sleep 0.6
shot 1500 '⏎|its answer'
click 8 15; sleep 0.4; press f; until_on 'only changes'; sleep 0.8
shot 1400 'f|its files and what changed'
for _ in 1 2 3; do press j; sleep 0.15; shot 180 'f|its files and what changed'; done
press Enter; until_on 'next/previous change'; sleep 0.5
shot 1400 '⏎|the diff, drawn by orcux'
for _ in 1 2 3 4 5 6 7; do press j; sleep 0.08; shot 110 '⏎|the diff, drawn by orcux'; done
press c; sleep 0.3
shot 400 'c|comment a line'
note='read the free-shipping limit from the config'
i=1
while [ $i -le ${#note} ]; do
    press -l "$(printf '%s' "$note" | cut -c$i-$((i + 2)))"; sleep 0.05
    shot 70 'c|comment a line'
    i=$((i + 3))
done
shot 600 'c|comment a line'
press Enter; until_on '1 comment'; sleep 0.3
shot 1000 'c|comment a line'
press q; until_on 'Give 1 comment'; sleep 0.3
shot 2000 'q|hand the comments to its agent'
press Enter; until_on 'free-shipping limit from the config'; sleep 1
shot 2200 '⏎|in its input, not sent: read it first'
press Escape; sleep 0.6; press P; until_on 'Open a pull request'; sleep 0.4
shot 2600 'P|open its pull request: asks first, on no'
press Escape; sleep 0.8

HOME=$real_home python3 "$here/render.py" "$frames" "$root/docs/demo.gif"
