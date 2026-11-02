# Windows ล็อก exe ที่กำลังรัน — rename หลบ (ไม่ kill process ของผู้ใช้)
$d = 'D:\cargo-target\skillhub\debug'
foreach ($n in 'mali_gate', 'mali_gate_ctl') {
  $exe = "$d\$n.exe"
  if (-not (Test-Path $exe)) { continue }
  try { [IO.File]::Open($exe, 'Open', 'ReadWrite', 'None').Close() } catch {
    $i = 0; while (Test-Path "$exe.locked$i") { try { Remove-Item "$exe.locked$i" -ErrorAction Stop } catch { $i++ } }
    Rename-Item $exe "$n.exe.locked$i"
  }
}
