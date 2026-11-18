# Stall watchdog (Linux, bash 4+). Kills a dependency fetch or build that has made no
# CPU or I/O progress for AO_STALL_SECS (default 180), so the step fails fast instead of
# hanging until the job timeout. Only cargo/pip/uv and npm-style installs are watched.
if [ -r /proc/self/stat ] && [ "${BASH_VERSINFO[0]:-0}" -ge 4 ] && [ "${AO_NO_WATCHDOG:-}" != 1 ] && command -v ps >/dev/null 2>&1; then
( _ao_root=$$; _ao_err="/proc/$$/fd/2"; exec 0</dev/null 1>/dev/null 2>/dev/null; _ao_lim=${AO_STALL_SECS:-180}; declare -A _ao_m _ao_t
  _ao_met() { local p=$1 s r w; s=$(awk '{print $14+$15}' "/proc/$p/stat" 2>/dev/null) || return 0; r=$(awk '/^(rchar|wchar):/{n+=$2} END{print n+0}' "/proc/$p/io" 2>/dev/null); echo $(( ${s:-0} + ${r:-0} )); }
  while kill -0 "$_ao_root" 2>/dev/null; do
    sleep 10
    declare -A _ao_par=() _ao_com=() _ao_arg=(); _ao_kids=()
    while read -r pid ppid comm args; do _ao_par[$pid]=$ppid; _ao_com[$pid]=$comm; _ao_arg[$pid]=$args; done < <(ps -eo pid=,ppid=,comm=,args= 2>/dev/null)
    now=$SECONDS
    for pid in "${!_ao_com[@]}"; do
      case "${_ao_com[$pid]}" in
        cargo) [[ " ${_ao_arg[$pid]} " =~ \ (build|fetch|check|update|install|clippy|add|vendor)\  ]] || continue ;;
        pip|pip3|uv) ;;
        npm|pnpm|yarn) [[ " ${_ao_arg[$pid]} " =~ \ (install|ci|add|i|update)\  ]] || continue ;;
        *) continue ;;
      esac
      a=$pid; ok=0; while [ "${a:-0}" -gt 1 ]; do [ "$a" = "$_ao_root" ] && { ok=1; break; }; a=${_ao_par[$a]:-0}; done
      [ "$ok" = 1 ] || continue
      tree=$pid; grow=1
      while [ "$grow" = 1 ]; do grow=0; for q in "${!_ao_par[@]}"; do case " $tree " in *" $q "*) ;; *) case " $tree " in *" ${_ao_par[$q]} "*) tree="$tree $q"; grow=1 ;; esac ;; esac; done; done
      m=0; for q in $tree; do m=$(( m + $(_ao_met "$q") )); done
      if [ "${_ao_m[$pid]:-x}" != "$m" ]; then _ao_m[$pid]=$m; _ao_t[$pid]=$now
      elif [ $(( now - ${_ao_t[$pid]:-$now} )) -ge "$_ao_lim" ]; then
        echo "[apollo] ${_ao_com[$pid]} made no CPU or network progress for ${_ao_lim}s; stopping it so the step fails fast (likely a stalled download). Run it again." >"$_ao_err" 2>/dev/null
        kill -TERM $tree 2>/dev/null; ( sleep 5; kill -KILL $tree 2>/dev/null ) & unset "_ao_m[$pid]" "_ao_t[$pid]"
      fi
    done
  done ) &
fi
