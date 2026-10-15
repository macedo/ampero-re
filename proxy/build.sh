#!/bin/sh
# Gera winmm.def (forward de tudo para winmm_real, exceto os hooks) e compila o proxy 32 bits.
set -e
cd "$(dirname "$0")"
GCC32=/c/Users/macedo/AppData/Local/Microsoft/WinGet/Packages/BrechtSanders.WinLibs.POSIX.MSVCRT_Microsoft.Winget.Source_8wekyb3d8bbwe/mingw32/bin
APP=../app
HOOKS="midiOutOpen@20 midiOutShortMsg@8 midiOutLongMsg@12 midiInOpen@20"
{
  echo "LIBRARY winmm.dll"
  echo "EXPORTS"
  "$GCC32/objdump" -p /c/Windows/SysWOW64/winmm.dll \
    | awk '/\[Ordinal\/Name Pointer\] Table/{p=1;next} p&&/^\s*\[/{print $NF} p&&/^$/{if(n++)exit}' \
    | while read -r name; do
        hook=""
        for h in $HOOKS; do [ "${h%@*}" = "$name" ] && hook="$h"; done
        if [ -n "$hook" ]; then echo "  $name=hk_$hook"; else echo "  $name=winmm_real.$name"; fi
      done
} > winmm.def
"$GCC32/gcc" -O2 -shared -s -static-libgcc -o winmm.dll winmm_proxy.c winmm.def -Wl,--kill-at
cp /c/Windows/SysWOW64/winmm.dll "$APP/winmm_real.dll"
cp winmm.dll "$APP/winmm.dll"
echo "ok -> $APP/winmm.dll"
