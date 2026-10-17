#!/usr/bin/env bash
set -euo pipefail
STATE=/run/hothash-bench.state
SYS=/sys/devices/system/cpu
WQ=/sys/devices/virtual/workqueue/cpumask
SLICE=hothashbench.slice

expand() {
  local part parts out=()
  IFS=, read -ra parts <<< "$1"
  for part in "${parts[@]}"; do
    if [[ $part == *-* ]]; then out+=($(seq "${part%-*}" "${part#*-}")); else out+=("$part"); fi
  done
  echo "${out[@]}"
}

sibling_of() {
  local c
  for c in $(expand "$(cat "$SYS/cpu$1/topology/thread_siblings_list")"); do
    [ "$c" != "$1" ] && echo "$c"
  done
  return 0
}

top_units() {
  local d
  for d in /sys/fs/cgroup/*.slice /sys/fs/cgroup/*.scope; do
    [ -d "$d" ] && [ "${d##*/}" != "$SLICE" ] && echo "${d##*/}"
  done
  return 0
}

pinned() {
  python3 -I - <<'PY'
import glob
def expand(s):
    out = []
    for p in s.split(","):
        a, _, b = p.partition("-")
        out += range(int(a), int(b or a) + 1)
    return out
for path in glob.glob("/proc/[0-9]*/task/[0-9]*/status"):
    try:
        f = dict(l.split(":\t", 1) for l in open(path).read().splitlines() if ":\t" in l)
    except OSError:
        continue
    cpus = expand(f["Cpus_allowed_list"].strip())
    if f.get("Kthread", "0").strip() != "1" and len(cpus) <= 4:
        for c in cpus:
            print(c, path.split("/")[4], f["Name"].strip())
PY
}

free() {
  local taken c s ok
  taken=" $(pinned | cut -d' ' -f1 | sort -un | tr '\n' ' ') "
  for c in $(expand "$(cat "$SYS/online")"); do
    s=$(expand "$(cat "$SYS/cpu$c/topology/thread_siblings_list")")
    [ "${s%% *}" = "$c" ] || continue
    ok=1
    for x in $s; do [[ $taken == *" $x "* ]] && ok=0; done
    [ "$ok" = 1 ] && echo "$c"
  done
  return 0
}

need_root() { [ "$(id -u)" = 0 ] || { echo "run as root: sudo $0 $*" >&2; exit 1; }; }

on() {
  local cpu="${1:-$(free | tail -1)}"
  [ -e "$STATE" ] && { echo "already on ($STATE exists); run: sudo $0 off" >&2; exit 1; }
  [ -d "$SYS/cpu$cpu" ] || { echo "no cpu$cpu" >&2; exit 1; }
  local sib others=() c mask=0 f u pid aff conflicts
  sib=$(sibling_of "$cpu")
  conflicts=$(pinned | awk -v a="$cpu" -v b="$sib" '$1 == a || $1 == b')
  if [ -n "$conflicts" ] && [ "${FORCE:-0}" != 1 ]; then
    echo "cpu$cpu/${sib:-} has pinned user threads (cpu tid name):" >&2
    echo "$conflicts" | head -5 >&2
    echo "free cores: $(free | tr '\n' ' ')(none means: stop whatever pins them)" >&2
    exit 1
  fi
  for c in $(expand "$(cat "$SYS/online")"); do
    if [ "$c" != "$cpu" ] && [ "$c" != "$sib" ]; then others+=("$c"); mask=$((mask | 1 << c)); fi
  done
  local list
  list=$(IFS=,; echo "${others[*]}")

  {
    echo "CPU=$cpu"
    echo "SIB=$sib"
    echo "GOV=$(cat "$SYS/cpu$cpu/cpufreq/scaling_governor")"
    echo "EPP=$(cat "$SYS/cpu$cpu/cpufreq/energy_performance_preference" 2>/dev/null || true)"
    echo "SMAX_WAS=$(cat "$SYS/cpu$cpu/cpufreq/scaling_max_freq")"
    echo "WQ_WAS=$(cat "$WQ" 2>/dev/null || true)"
    echo "UNITS=\"$(top_units | tr '\n' ' ')\""
    for f in /proc/irq/[0-9]*/smp_affinity_list; do echo "IRQ $f $(cat "$f")"; done
  } > "$STATE"

  [ -n "$sib" ] && echo 0 > "$SYS/cpu$sib/online"
  for u in $(top_units); do systemctl set-property --runtime "$u" AllowedCPUs="$list"; done
  local irqs=0 kts=0
  for f in /proc/irq/[0-9]*/smp_affinity_list; do
    echo "$list" 2>/dev/null > "$f" && irqs=$((irqs + 1))
  done
  for pid in $(cat /sys/fs/cgroup/cgroup.procs); do
    aff=$(taskset -pc "$pid" 2>/dev/null) || continue
    if taskset -pc "$list" "$pid" > /dev/null 2>&1; then
      echo "KT $pid ${aff##*: }" >> "$STATE"
      kts=$((kts + 1))
    fi
  done
  [ -e "$WQ" ] && printf '%x\n' "$mask" > "$WQ"
  echo performance > "$SYS/cpu$cpu/cpufreq/scaling_governor"
  if [ "${BOOST:-0}" != 1 ]; then
    echo $(($(cat "$SYS/cpu$cpu/acpi_cppc/nominal_freq") * 1000)) > "$SYS/cpu$cpu/cpufreq/scaling_max_freq"
  fi
  echo "on: cpu$cpu isolated (sibling ${sib:-none} offline); other cgroups, $irqs IRQs and $kts kernel threads on $list;" \
    "max $(($(cat "$SYS/cpu$cpu/cpufreq/scaling_max_freq") / 1000)) MHz" >&2
}

off() {
  [ -e "$STATE" ] || { echo "not on (no $STATE)" >&2; return 0; }
  local CPU SIB GOV EPP SMAX_WAS WQ_WAS UNITS u
  source <(grep -E '^(CPU|SIB|GOV|EPP|SMAX_WAS|WQ_WAS|UNITS)=' "$STATE")
  [ -n "$SIB" ] && echo 1 > "$SYS/cpu$SIB/online"
  for u in $UNITS; do systemctl set-property --runtime "$u" AllowedCPUs= || true; done
  grep '^IRQ ' "$STATE" | while read -r _ f aff; do echo "$aff" > "$f" 2>/dev/null || true; done
  grep '^KT ' "$STATE" | while read -r _ pid aff; do taskset -pc "$aff" "$pid" > /dev/null 2>&1 || true; done
  [ -n "$WQ_WAS" ] && echo "$WQ_WAS" > "$WQ"
  [ -n "$SMAX_WAS" ] && echo "$SMAX_WAS" > "$SYS/cpu$CPU/cpufreq/scaling_max_freq"
  echo "$GOV" > "$SYS/cpu$CPU/cpufreq/scaling_governor"
  [ -n "$EPP" ] && { echo "$EPP" > "$SYS/cpu$CPU/cpufreq/energy_performance_preference" 2>/dev/null || true; }
  systemctl stop "$SLICE" 2>/dev/null || true
  rm -f "$STATE"
  echo "off: restored" >&2
}

run() {
  local cpu="$1"
  shift
  [ "${1:-}" = -- ] && shift
  [ -n "${SUDO_UID:-}" ] || { echo "start it with sudo, so CMD runs as you" >&2; exit 1; }
  if [ -e "$STATE" ]; then
    [ "$(sed -n 's/^CPU=//p' "$STATE")" = "$cpu" ] || { echo "cpu$(sed -n 's/^CPU=//p' "$STATE") is isolated; sudo $0 off first" >&2; exit 1; }
  else
    trap off EXIT
    trap 'exit 130' INT TERM
    on "$cpu"
  fi
  systemd-run --scope --quiet --collect --slice="$SLICE" -p AllowedCPUs="$cpu" -- \
    setpriv --reuid="$SUDO_UID" --regid="$SUDO_GID" --init-groups -- "$@"
}

status() {
  local cpu sib on=false sib_online=none
  if [ -e "$STATE" ]; then
    on=true
    cpu=$(sed -n 's/^CPU=//p' "$STATE")
    sib=$(sed -n 's/^SIB=//p' "$STATE")
  else
    cpu="${1:-$(free | tail -1)}"
    sib=$(sibling_of "$cpu")
  fi
  [ -n "$sib" ] && sib_online=$(cat "$SYS/cpu$sib/online")
  local gov maxf freq allowed
  gov=$(cat "$SYS/cpu$cpu/cpufreq/scaling_governor")
  maxf=$(($(cat "$SYS/cpu$cpu/cpufreq/scaling_max_freq") / 1000))
  freq=$(cat "$SYS/cpu$cpu/cpufreq/cpuinfo_avg_freq" 2>/dev/null || cat "$SYS/cpu$cpu/cpufreq/scaling_cur_freq")
  allowed=$(cat /sys/fs/cgroup/user.slice/cpuset.cpus.effective 2>/dev/null || echo "?")
  if [ "${JSON:-0}" = 1 ]; then
    printf '{"isolated":%s,"cpu":%s,"sibling":"%s","sibling_online":"%s","governor":"%s","max_freq_mhz":%s,"user_slice_cpus":"%s"}\n' \
      "$on" "$cpu" "$sib" "$sib_online" "$gov" "$maxf" "$allowed"
  else
    echo "isolated=$on cpu=$cpu sibling=${sib:-none} sibling_online=$sib_online governor=$gov max_freq=${maxf}MHz avg_freq=${freq}kHz user.slice cpus=$allowed"
  fi
}

case "${1:-}" in
  run) need_root run; [ -n "${2:-}" ] || { echo "usage: sudo $0 run CPU -- CMD..." >&2; exit 1; }; run "${@:2}" ;;
  on) need_root on; on "${2:-}"; status ;;
  off) need_root off; off ;;
  free) free ;;
  status) if [ "${2:-}" = --json ]; then JSON=1 status "${3:-}"; else status "${2:-}"; fi ;;
  *) printf 'usage: sudo %s run CPU -- CMD...\n       sudo %s on [CPU]\n       sudo %s off\n       %s status [--json]\n       %s free\n' "$0" "$0" "$0" "$0" "$0" >&2; exit 1 ;;
esac
