# Oracle: list-page payload for the agents view (slice 1).
#
# Source-of-truth strategy: TRACKING-FIRST.
#   1. Read .kiro/installed-agents.json -> tracked agent names.
#   2. For each tracked name, read the file (skip if missing); attach lineage.
#   3. For each *.json file NOT in tracking, append a row with null lineage.
#   4. Sort union by name and emit JSON.
#
# Independent from probe.py: different runtime (PowerShell vs CPython),
# different JSON parser (System.Text.Json vs json module), different
# iteration model (cmdlet pipeline vs Path.glob). If probe and oracle
# disagree on output, the data model in the spec has a hidden assumption.
#
# Usage:  pwsh -File oracle.ps1 <project_path>

param([string]$ProjectPath = ".")

$ErrorActionPreference = "Stop"

$track  = Join-Path $ProjectPath ".kiro/installed-agents.json"
$agents = Join-Path $ProjectPath ".kiro/agents"

# Tracked map: name -> {marketplace, plugin, version}
$tracking = @{}
if (Test-Path $track) {
    $raw = Get-Content $track -Raw | ConvertFrom-Json
    if ($raw.agents) {
        foreach ($prop in $raw.agents.PSObject.Properties) {
            $tracking[$prop.Name] = $prop.Value
        }
    }
}

function New-Row {
    param($Agent, [string]$Name, $Lineage)
    $hooksCount = 0
    if ($Agent.hooks) {
        foreach ($p in $Agent.hooks.PSObject.Properties) {
            if ($p.Value -is [System.Array]) { $hooksCount += $p.Value.Length }
        }
    }
    $mcpCount = 0
    if ($Agent.mcpServers) { $mcpCount = @($Agent.mcpServers.PSObject.Properties).Count }
    [ordered]@{
        name             = $Name
        description      = $Agent.description
        model            = $Agent.model
        tools_count      = @($Agent.tools).Where({ $_ }).Count
        mcp_count        = $mcpCount
        resources_count  = @($Agent.resources).Where({ $_ }).Count
        hooks_count      = $hooksCount
        lineage          = $Lineage
    }
}

$rows = @()

# Step 1: tracked-first, file must exist.
foreach ($name in $tracking.Keys) {
    $jf = Join-Path $agents "$name.json"
    if (-not (Test-Path $jf)) { continue }
    $agent = Get-Content $jf -Raw | ConvertFrom-Json
    $t = $tracking[$name]
    $lineage = [ordered]@{
        marketplace = $t.marketplace
        plugin      = $t.plugin
        version     = $t.version
    }
    $rows += New-Row -Agent $agent -Name $name -Lineage $lineage
}

# Step 2: untracked files (filesystem-side complement).
if (Test-Path $agents) {
    foreach ($jf in Get-ChildItem -Path $agents -Filter *.json) {
        $base = [System.IO.Path]::GetFileNameWithoutExtension($jf.Name)
        if ($tracking.ContainsKey($base)) { continue }
        $agent = Get-Content $jf.FullName -Raw | ConvertFrom-Json
        $rows += New-Row -Agent $agent -Name $base -Lineage $null
    }
}

$rows | Sort-Object -Property { $_.name } | ConvertTo-Json -Depth 6
