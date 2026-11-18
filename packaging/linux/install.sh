#!/bin/sh
# Installs MD++ for the current user (or under PREFIX): the program, its menu entry, icon and
# the association with Markdown files. Run from the unpacked release folder.
#   ./install.sh              → ~/.local
#   PREFIX=/usr/local sudo -E ./install.sh
#   ./install.sh --uninstall
set -eu
PREFIX="${PREFIX:-$HOME/.local}"
ID=io.github.mcftc.mdpp
here=$(cd "$(dirname "$0")" && pwd)
files="bin/mdpp share/applications/$ID.desktop share/icons/hicolor/scalable/apps/$ID.svg share/metainfo/$ID.metainfo.xml"

if [ "${1:-}" = "--uninstall" ]; then
    for f in $files; do rm -f "$PREFIX/$f"; done
    echo "MD++ removed from $PREFIX (settings stay in ~/.config/mdpp)."
    exit 0
fi

install -Dm755 "$here/mdpp" "$PREFIX/bin/mdpp"
install -Dm644 "$here/$ID.desktop" "$PREFIX/share/applications/$ID.desktop"
install -Dm644 "$here/$ID.svg" "$PREFIX/share/icons/hicolor/scalable/apps/$ID.svg"
install -Dm644 "$here/$ID.metainfo.xml" "$PREFIX/share/metainfo/$ID.metainfo.xml"
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$PREFIX/share/applications" || true
command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q "$PREFIX/share/icons/hicolor" >/dev/null 2>&1 || true
echo "MD++ installed in $PREFIX/bin/mdpp; it is in the application menu and opens .md files."
