# S13 contract oracle (independent of probe.mjs).
#
# Reads the Rust source files that specta generated bindings.ts
# from. If specta is correct (verified by C8 idempotent regen), a
# match between this oracle's output and probe.mjs's output proves
# the bindings + helpers I'll write S13 against are actually the
# contract the backend exposes.
#
# Different mechanism than the probe:
#   probe.mjs : Node.js text reads on TypeScript output
#   oracle.ps1: PowerShell text reads on Rust source input
#
# Same answer should fall out either way.

param([string]$Root = ".")

function Show-Section($title) {
    Write-Host ""
    Write-Host "# $title"
    Write-Host ""
}

Write-Host "=== S13 contract oracle (Rust source, upstream of specta) ==="

Show-Section "1. SaveOutcome (kiro-market-core/src/user_agent.rs)"
Get-Content "$Root\crates\kiro-market-core\src\user_agent.rs" |
    Select-String -Pattern "^pub struct SaveOutcome" -Context 0,8 |
    ForEach-Object { $_.Line; $_.Context.PostContext -join "`n" }

Show-Section "2. UserAgentRow (user_agent.rs)"
Get-Content "$Root\crates\kiro-market-core\src\user_agent.rs" |
    Select-String -Pattern "^pub struct UserAgentRow" -Context 0,12 |
    ForEach-Object { $_.Line; $_.Context.PostContext -join "`n" }

Show-Section "3. UserAgentLineage (user_agent.rs)"
Get-Content "$Root\crates\kiro-market-core\src\user_agent.rs" |
    Select-String -Pattern "^pub struct UserAgentLineage" -Context 0,8 |
    ForEach-Object { $_.Line; $_.Context.PostContext -join "`n" }

Show-Section "4. ErrorType variants (kiro-control-center/src-tauri/src/error.rs)"
Get-Content "$Root\crates\kiro-control-center\src-tauri\src\error.rs" |
    Select-String -Pattern "^\s*(NotFound|AlreadyExists|Validation|GitError|IoError|ParseError|Internal|Unknown)\s*,?\s*$"

Show-Section "5. The 5 #[tauri::command] signatures (commands/agents_authoring.rs)"
Get-Content "$Root\crates\kiro-control-center\src-tauri\src\commands\agents_authoring.rs" |
    Select-String -Pattern "^pub async fn (list_user_agents|create_user_agent|save_user_agent|delete_user_agent|duplicate_user_agent)" -Context 0,1 |
    ForEach-Object { $_.Line; $_.Context.PostContext -join "`n"; "---" }

Show-Section "6. AgentsTabMode consumer parity (AgentsTab.svelte uses kind values list/new/edit)"
Select-String -Path "$Root\crates\kiro-control-center\src\lib\components\AgentsTab.svelte" `
    -Pattern 'mode\.kind === "(list|new|edit)"|mode = \{ kind: "(list|new|edit)"' |
    ForEach-Object { "L$($_.LineNumber): $($_.Line.Trim())" }
