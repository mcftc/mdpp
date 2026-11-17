#!/usr/bin/env bash
# usage: measure.sh <name> <binary> <file>   -> prints size, first-frame ms, RSS, idle CPU; saves screenshot
name=$1; bin=$2; file=$3; out=$(dirname "$0")/results; mkdir -p "$out"
size=$(stat -c %s "$bin")
"$bin" "$file" 2> "$out/$name.stderr" & pid=$!
sleep 6
rss1=$(awk '/VmRSS/{print $2}' /proc/$pid/status)
t1=$(awk '{print $14+$15}' /proc/$pid/stat); sleep 10; t2=$(awk '{print $14+$15}' /proc/$pid/stat)
rss2=$(awk '/VmRSS/{print $2}' /proc/$pid/status)
hz=$(getconf CLK_TCK)
sleep 0
kill $pid 2>/dev/null; wait $pid 2>/dev/null
python3 -c "import sys; n,s,r1,r2,t1,t2,hz=sys.argv[1:8]; print(f\"{n:5} size={int(s)/1048576:.1f}MB rss6s={int(r1)//1024}MB rss16s={int(r2)//1024}MB idle_cpu={(int(t2)-int(t1))*100/int(hz)/10:.2f}%\", end=\" \")" "$name" "$size" "$rss1" "$rss2" "$t1" "$t2" "$hz"; grep -hE "MS " "$out/$name.stderr" | tr "\n" " "; echo
