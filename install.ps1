# install.ps1 — Windows installer stub for ArcanaGlyph.
#
# Usage (when supported):
#   powershell -ExecutionPolicy ByPass -c "irm https://github.com/py-art/arcanaglyph/raw/main/install.ps1 | iex"

Write-Host ""
Write-Host "ArcanaGlyph does not support Windows yet." -ForegroundColor Yellow
Write-Host ""
Write-Host "Linux x86_64 is the only supported platform at this time."
Write-Host "The application relies on Linux-specific subsystems (Wayland/X11"
Write-Host "input injection, XDG portals, ALSA/PipeWire) that have no Windows"
Write-Host "equivalent without a full port."
Write-Host ""
Write-Host "Track progress or open an issue:"
Write-Host "  https://github.com/py-art/arcanaglyph"
Write-Host ""
exit 1
