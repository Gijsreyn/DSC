# Copyright (c) Microsoft Corporation.
# Licensed under the MIT License.

Describe 'PowerShell adapter resource tests' {

    BeforeAll {
        $OldPSModulePath  = $env:PSModulePath
        $env:PSModulePath += [System.IO.Path]::PathSeparator + $PSScriptRoot

        if ($IsLinux -or $IsMacOS) {
            $cacheFilePath = Join-Path $env:HOME ".dsc" "PSAdapterCache.json"
        }
        else
        {
            $cacheFilePath = Join-Path $env:LocalAppData "dsc" "PSAdapterCache.json"
        }
    }
    AfterAll {
        $env:PSModulePath = $OldPSModulePath
    }

    BeforeEach {
        Remove-Item -Force -ea SilentlyContinue -Path $cacheFilePath
    }

    It 'Discovery includes class-based resources' {

        $r = dsc resource list '*' -a Microsoft.DSC/PowerShell
        $LASTEXITCODE | Should -Be 0
        $resources = $r | ConvertFrom-Json
        ($resources | ? {$_.Type -eq 'TestClassResource/TestClassResource'}).Count | Should -Be 1
    }

    It 'Get works on class-based resource' {

        $r = "{'Name':'TestClassResource1'}" | dsc resource get -r 'TestClassResource/TestClassResource'
        $LASTEXITCODE | Should -Be 0
        $res = $r | ConvertFrom-Json
        $res.actualState.result.properties.Prop1 | Should -BeExactly 'ValueForProp1'
    }

    It 'Get uses enum names on class-based resource' {

        $r = "{'Name':'TestClassResource1'}" | dsc resource get -r 'TestClassResource/TestClassResource'
        $LASTEXITCODE | Should -Be 0
        $res = $r | ConvertFrom-Json
        $res.actualState.result.properties.EnumProp | Should -BeExactly 'Expected'
    }

    It 'Test works on class-based resource' {

        $r = "{'Name':'TestClassResource1','Prop1':'ValueForProp1'}" | dsc resource test -r 'TestClassResource/TestClassResource'
        $LASTEXITCODE | Should -Be 0
        $res = $r | ConvertFrom-Json
        $res.actualState.result.properties.InDesiredState | Should -Be $True
    }

    It 'Set works on class-based resource' {

        $r = "{'Name':'TestClassResource1','Prop1':'ValueForProp1'}" | dsc resource set -r 'TestClassResource/TestClassResource'
        $LASTEXITCODE | Should -Be 0
        $res = $r | ConvertFrom-Json
        $res.afterState.result | Should -Not -BeNull
    }

    It 'Export works on PS class-based resource' {

        $r = dsc resource export -r TestClassResource/TestClassResource
        $LASTEXITCODE | Should -Be 0
        $res = $r | ConvertFrom-Json
        $res.resources[0].properties.result.count | Should -Be 5
        $res.resources[0].properties.result[0].Name | Should -Be "Object1"
        $res.resources[0].properties.result[0].Prop1 | Should -Be "Property of object1"
    }

    It 'Get --all works on PS class-based resource' {

        $r = dsc resource get --all -r TestClassResource/TestClassResource
        $LASTEXITCODE | Should -Be 0
        $res = $r | ConvertFrom-Json
        $res.actualState.result.count | Should -Be 5
        $res.actualState.result| % {$_.Name | Should -Not -BeNullOrEmpty}
    }

    It 'Verify that ClearCache works in PSAdapter' {
        # generate the cache
        $null = dsc resource list '*' -a Microsoft.DSC/PowerShell
        # call the ClearCache operation
        $scriptPath = Join-Path $PSScriptRoot '..' 'psDscAdapter' 'powershell.resource.ps1'
        $null = & $scriptPath -Operation ClearCache
        # verify that PSAdapter does not find the cache
        dsc -l debug resource list '*' -a Microsoft.DSC/PowerShell 2> $TestDrive/tracing.txt
        $LASTEXITCODE | Should -Be 0
        "$TestDrive/tracing.txt" | Should -FileContentMatchExactly 'Cache file not found'
    }

    It 'Verify that a new PS Cache version results in cache rebuid' {
        # generate the cache
        $null = dsc resource list '*' -a Microsoft.DSC/PowerShell
        # update the version in the cache file
        $cacheFilePath = if ($IsWindows) {
            # PS 6+ on Windows
            Join-Path $env:LocalAppData "dsc\PSAdapterCache.json"
        } else {
            # either WinPS or PS 6+ on Linux/Mac
            if ($PSVersionTable.PSVersion.Major -le 5) {
                Join-Path $env:LocalAppData "dsc\WindowsPSAdapterCache.json"
            } else {
                Join-Path $env:HOME ".dsc" "PSAdapterCache.json"
            }
        }
        $cache = Get-Content -Raw $cacheFilePath | ConvertFrom-Json
        $cache.CacheSchemaVersion = 0
        $jsonCache = $cache | ConvertTo-Json -Depth 90
        New-Item -Force -Path $cacheFilePath -Value $jsonCache -Type File | Out-Null

        # verify that a new PS Cache version results in cache rebuid
        dsc -l debug resource list '*' -a Microsoft.DSC/PowerShell 2> $TestDrive/tracing.txt
        $LASTEXITCODE | Should -Be 0
        "$TestDrive/tracing.txt" | Should -FileContentMatchExactly 'Incompatible version of cache in file'
    }

    It 'Verify inheritance works in class-based resources' {

        $r = dsc resource list '*' -a Microsoft.DSC/PowerShell
        $LASTEXITCODE | Should -Be 0
        $resources = $r | ConvertFrom-Json
        $t = $resources | ? {$_.Type -eq 'TestClassResource/TestClassResource'}
        $t.properties | Should -Contain "BaseProperty"
    }

    It 'Verify highest module version is loaded' {

        $srcPath = Join-Path $PSScriptRoot 'TestClassResource'
        $pathRoot1 = Join-Path $TestDrive 'A'
        $pathRoot2 = Join-Path $TestDrive 'B'
        $path1 = Join-Path $pathRoot1 'TestClassResource' '0.0.1'
        $path2 = Join-Path $pathRoot1 'TestClassResource' '0.0.2'
        $path3 = Join-Path $pathRoot2 'TestClassResource' '0.0.3'
        $path4 = Join-Path $pathRoot2 'TestClassResource' '0.0.4'

        New-Item -ItemType Directory -Force -Path $path1 | Out-Null
        New-Item -ItemType Directory -Force -Path $path2 | Out-Null
        New-Item -ItemType Directory -Force -Path $path3 | Out-Null
        New-Item -ItemType Directory -Force -Path $path4 | Out-Null

        $files = Get-ChildItem -Recurse -File -Path $srcPath
        $files | Copy-Item -Destination $path1
        $files | Copy-Item -Destination $path2
        $files | Copy-Item -Destination $path3
        $files | Copy-Item -Destination $path4

        $filePath = Join-Path $path1 'TestClassResource.psd1'
        (Get-Content -Raw $filePath).Replace("ModuleVersion = `'0.0.1`'", "ModuleVersion = `'0.0.1`'") | Set-Content $filePath
        $filePath = Join-Path $path2 'TestClassResource.psd1'
        (Get-Content -Raw $filePath).Replace("ModuleVersion = `'0.0.1`'", "ModuleVersion = `'0.0.2`'") | Set-Content $filePath
        $filePath = Join-Path $path3 'TestClassResource.psd1'
        (Get-Content -Raw $filePath).Replace("ModuleVersion = `'0.0.1`'", "ModuleVersion = `'0.0.3`'") | Set-Content $filePath
        $filePath = Join-Path $path4 'TestClassResource.psd1'
        (Get-Content -Raw $filePath).Replace("ModuleVersion = `'0.0.1`'", "ModuleVersion = `'0.0.4`'") | Set-Content $filePath


        $oldPath = $env:PSModulePath
        try {
            $env:PSModulePath += [System.IO.Path]::PathSeparator + $pathRoot1
            $env:PSModulePath += [System.IO.Path]::PathSeparator + $pathRoot2

            $r = dsc resource list '*' -a Microsoft.DSC/PowerShell
            $LASTEXITCODE | Should -Be 0
            $resources = $r | ConvertFrom-Json
            $r = @($resources | ? {$_.Type -eq 'TestClassResource/TestClassResource'})
            $r.Count | Should -Be 1
            $r[0].Version | Should -Be '0.0.4'
        }
        finally {
            $env:PSModulePath = $oldPath
        }

    }
}
