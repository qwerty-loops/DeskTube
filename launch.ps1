$root = "C:\Users\Allen\Documents\Central\Full_time_prep\Learning\Desk_Tube"
$exe = "$root\src-tauri\target\debug\desk-tube.exe"

function Test-Port1420 {
    [bool](Get-NetTCPConnection -LocalPort 1420 -State Listen -ErrorAction SilentlyContinue)
}

# Start a static server for the frontend only if nothing is already on 1420.
$server = $null
if (-not (Test-Port1420)) {
    $server = Start-Process -PassThru -WindowStyle Hidden -FilePath "pnpm.cmd" `
        -ArgumentList "preview --port 1420 --strictPort" -WorkingDirectory $root
}

$waited = 0
while (-not (Test-Port1420) -and $waited -lt 40) {
    Start-Sleep -Milliseconds 500
    $waited++
}

try {
    & $exe | Out-Null
}
finally {
    # Stop the preview server we started once the app closes.
    if ($server -and -not $server.HasExited) {
        Stop-Process -Id $server.Id -Force -ErrorAction SilentlyContinue
    }
}
