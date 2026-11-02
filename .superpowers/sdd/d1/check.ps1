# fmt + clippy + test; logs kept in this folder; prints exit codes + failing tests only
$d = $PSScriptRoot
& (Join-Path $PSScriptRoot "unlock.ps1")
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings *> "$d\clippy.log"; $c = $LASTEXITCODE
cargo test --workspace *> "$d\test.log"; $t = $LASTEXITCODE
"clippy exit=$c  test exit=$t"
if ($c -ne 0) { Get-Content "$d\clippy.log" | Select-String -CaseSensitive -Pattern "^(error|warning)|-->" | Select-Object -First 20 }
$sum = 0; Get-Content "$d\test.log" | Select-String -Pattern "test result: \w+\. (\d+) passed" | ForEach-Object { $sum += [int]$_.Matches[0].Groups[1].Value }
"tests passed total: $sum"
Get-Content "$d\test.log" | Select-String -CaseSensitive -Pattern "FAILED|panicked|^error" | Select-Object -First 20
