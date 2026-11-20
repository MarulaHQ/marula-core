# install.ps1 -- the one-line entrance to a Marula release, on Windows.
#
#   irm https://github.com/MarulaHQ/marula-core/releases/latest/download/install.ps1 | iex
#   & ([scriptblock]::Create((irm https://github.com/MarulaHQ/marula-core/releases/latest/download/install.ps1))) -Version 0.6.1
#   & ([scriptblock]::Create((irm https://github.com/MarulaHQ/marula-core/releases/latest/download/install.ps1))) -Version 0.6.1 --dir C:\srv\marula --apply
#
# The PowerShell half of scripts/release/bootstrap-install.sh, and attached to
# every release beside it. Same order, same refusals: work out the platform,
# download that platform's archive, its checksum list and the signature over
# that list, verify the signature against the release key pinned below, verify
# the archive against the list, unpack it into a directory it keeps, and hand
# over to the bundle's installer. Anything it does not recognise is passed
# through unchanged.
#
# There is no param() block on purpose. `irm | iex` runs this text in the
# caller's own scope, where a parameter block would bind against nothing, so the
# arguments arrive in $args and are parsed below. That keeps one script working
# for both entrance forms: `iex`, and `& ([scriptblock]::Create(...)) -Version x`.
#
# Why the key is pinned in this file rather than fetched: the release key is the
# only thing that makes a download trustworthy, so it cannot come down the same
# channel as the download. It is the same key as `release-keys/allowed_signers`.
#
# Non-purpose: it does not install Docker, and it does not continue when
# `ssh-keygen` is missing, because a missing tool is not a reason to trust a
# download. Windows 10 1809 and later ship the OpenSSH client with `ssh-keygen`
# in it; a machine without that feature is refused, not trusted.

# The body lives in one scriptblock so `iex` does not leave the helpers and the
# variables behind in the caller's session. The caller's arguments are copied
# out first: inside the block, $args is the block's own (empty) argument list.
$MarulaBootstrapArgs = @($args)
& {
    $ErrorActionPreference = 'Stop'
    # PowerShell 7.3 and later can turn a non-zero exit from a native command
    # into a terminating error. Every native command below is checked through
    # $LASTEXITCODE instead, so that the message names what failed; this turns
    # the shorthand off where it exists.
    if (Test-Path variable:PSNativeCommandUseErrorActionPreference) {
        $PSNativeCommandUseErrorActionPreference = $false
    }
    # Windows PowerShell 5.1 on an older Windows 10 build can offer TLS 1.0
    # first, which GitHub refuses. pwsh 7 already negotiates TLS 1.2 and this is
    # a no-op there.
    try {
        [System.Net.ServicePointManager]::SecurityProtocol = [System.Net.ServicePointManager]::SecurityProtocol -bor [System.Net.SecurityProtocolType]::Tls12
    }
    catch {
    }

    $Repository = 'MarulaHQ/marula-core'
    $KeyId = 'releases@getmarula.com'
    $KeyLine = 'releases@getmarula.com ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIOJw34oaF4eoSLXMzPpVLpXkXGOtAYEnE/ZhY9/hItn9 releases@getmarula.com'
    $KeyFingerprint = 'SHA256:F1n6L6KfpRiPy8tF0Y465VWobIVrYPExwseUefv4Heg'

    function Say([string]$Message) {
        Write-Host "==> $Message"
    }

    # A throw rather than `exit`: under `irm | iex` an exit would close the
    # caller's PowerShell window, and a failed download should not cost someone
    # their session. Run with `pwsh -File`, an unhandled throw exits 1.
    function Fail([string]$Message) {
        throw "install: $Message"
    }

    function Show-Usage {
        @'
Usage: install.ps1 [-Version <X.Y.Z>] [-Offline] [-Help] [-- <installer args>]

Downloads the Marula release for this machine, checks its signature, unpacks it
and runs the bundle's installer. With no installer arguments that opens the
wizard and installs into %USERPROFILE%\marula.

  -Version <X.Y.Z>   Install this release instead of the newest one.
  -Offline           Refused: the air-gapped bundle is published for Linux only.
  -Dir <path>        Passed to the installer: which directory to install into.
  -Apply             Passed to the installer: write the env file and start.

Anything else is passed straight through, with or without a `--` before it. The
installer's own flags can be written the PowerShell way (`-Apply`, `-NoOpen`) or
the way its documentation does (`--apply`, `--no-open`); both arrive as the flag
the CLI knows.
  install.ps1 -Version 0.6.1 -Dir C:\srv\marula -Apply
'@ | Write-Host
    }

    # The caller's own directory, captured before anything moves. A relative
    # -Dir is relative to *their* shell, which is the rule deploy/install.sh
    # follows on the other platforms.
    $CallerLocation = (Get-Location).ProviderPath

    function Resolve-FullPath([string]$Path) {
        # Path.GetFullPath(string, string) is .NET Core only, and Windows
        # PowerShell 5.1 runs on .NET Framework, so a relative path is joined
        # against the caller's directory first.
        if ([System.IO.Path]::IsPathRooted($Path)) {
            return [System.IO.Path]::GetFullPath($Path)
        }
        return [System.IO.Path]::GetFullPath((Join-Path $CallerLocation $Path))
    }

    # A PowerShell caller writes `-Apply` and the CLI spells it `--apply`. One
    # dash and a capital letter is that spelling, and it is translated:
    # -NoOpen becomes --no-open, -MemoryMb becomes --memory-mb. Everything else
    # is passed through untouched, so the CLI's own documented flags keep
    # working exactly as written.
    function ConvertTo-InstallerArgument([string]$Argument) {
        if ($Argument -cmatch '^-[A-Z][A-Za-z0-9]*$') {
            $Name = $Argument.Substring(1)
            $Kebab = ($Name -creplace '([a-z0-9])([A-Z])', '$1-$2').ToLowerInvariant()
            return "--$Kebab"
        }
        return $Argument
    }

    $Version = if ($env:MARULA_VERSION) { $env:MARULA_VERSION } else { 'latest' }
    $Offline = $false
    $PassThrough = New-Object System.Collections.Generic.List[string]

    $Index = 0
    while ($Index -lt $MarulaBootstrapArgs.Count) {
        $Argument = [string]$MarulaBootstrapArgs[$Index]
        if ($Argument -eq '--version' -or $Argument -eq '-Version') {
            if ($Index + 1 -ge $MarulaBootstrapArgs.Count) { Fail '-Version needs a value' }
            $Version = [string]$MarulaBootstrapArgs[$Index + 1]
            $Index += 2
        }
        elseif ($Argument -like '--version=*') {
            $Version = $Argument.Substring('--version='.Length)
            $Index += 1
        }
        elseif ($Argument -eq '--offline' -or $Argument -eq '-Offline' -or $Argument -eq '-offline') {
            $Offline = $true
            $Index += 1
        }
        elseif ($Argument -eq '--help' -or $Argument -eq '-Help' -or $Argument -eq '-h' -or $Argument -eq '/?') {
            Show-Usage
            return
        }
        elseif ($Argument -eq '--') {
            # Everything after this belongs to the installer, including the
            # tokens that look like the flags above.
            $Index += 1
            break
        }
        else {
            break
        }
    }
    while ($Index -lt $MarulaBootstrapArgs.Count) {
        $PassThrough.Add([string]$MarulaBootstrapArgs[$Index])
        $Index += 1
    }

    # `v0.6.1` and `0.6.1` are both things a user will type, and both have to
    # name the same archive.
    if ($Version.StartsWith('v')) { $Version = $Version.Substring(1) }

    $Architecture = $env:PROCESSOR_ARCHITECTURE
    switch ($Architecture) {
        'AMD64' { $Arch = 'x64' }
        'ARM64' {
            Fail 'the release publishes a windows-x64 build only; there is no windows-arm64 archive to install'
        }
        default {
            Fail "no published build for this architecture ($Architecture)"
        }
    }
    $Os = 'windows'

    if ($Offline) {
        Fail 'the air-gapped bundle is published for Linux only. Download marula-<version>-offline-linux-<arch>.tar.zst on the Linux host that will run the stack'
    }

    # A mirror can repoint the downloads without touching the script, and that
    # is safe because the signature below is pinned: a mirror cannot substitute
    # bytes. It is also what a test fixture points at.
    $ReleasesUrl = if ($env:MARULA_RELEASES_URL) { $env:MARULA_RELEASES_URL.TrimEnd('/') } else { "https://github.com/$Repository/releases" }

    function Get-RemoteFile([string]$Uri, [string]$Destination) {
        # Three attempts, like the shell script's `curl --retry 3`: a release
        # download over a flaky link is the normal case, not the exception.
        $Attempts = 3
        for ($Attempt = 1; $Attempt -le $Attempts; $Attempt++) {
            try {
                Invoke-WebRequest -Uri $Uri -OutFile $Destination -UseBasicParsing -MaximumRedirection 5 -ErrorAction Stop
                return
            }
            catch {
                if ($Attempt -eq $Attempts) {
                    Fail "could not download $Uri ($($_.Exception.Message))"
                }
                Start-Sleep -Seconds 2
            }
        }
    }

    # "latest" cannot name an asset, because asset names carry the version. The
    # release page redirect ends at /tag/vX.Y.Z, which is where the version can
    # be read; asset URLs leave for the CDN and lose the tag on the way.
    function Get-NewestVersion {
        Say 'resolving the newest release'
        # The redirect chain is walked with HttpClient and redirection off: the
        # tag is in the address the release page forwards to, and the page body
        # is not what this needs, so it is never downloaded. Invoking this
        # through Invoke-WebRequest and reading the final URL from the response
        # would work too, but it fetches the whole page and the property that
        # carries the URL is named differently on 5.1 and 7.
        $Handler = New-Object System.Net.Http.HttpClientHandler
        $Handler.AllowAutoRedirect = $false
        $Client = New-Object System.Net.Http.HttpClient($Handler)
        $Client.Timeout = [TimeSpan]::FromSeconds(30)
        $Uri = [System.Uri]"$ReleasesUrl/latest"
        try {
            for ($Hop = 0; $Hop -lt 5; $Hop++) {
                $Response = $Client.GetAsync($Uri).GetAwaiter().GetResult()
                try {
                    $Location = $Response.Headers.Location
                    if (-not $Location) { break }
                    if (-not $Location.IsAbsoluteUri) {
                        $Location = New-Object System.Uri($Uri, $Location)
                    }
                    $Uri = $Location
                }
                finally {
                    $Response.Dispose()
                }
            }
        }
        catch {
            Fail "could not resolve the newest release ($($_.Exception.Message)); pass -Version"
        }
        finally {
            $Client.Dispose()
        }
        if ($Uri.AbsoluteUri -match '/tag/v([^/]+)/?$') { return $Matches[1] }
        Fail "could not read a version from $($Uri.AbsoluteUri); pass -Version"
    }

    if ($Version -eq 'latest') {
        $Version = Get-NewestVersion
        Say "newest release is $Version"
    }

    $Base = "$ReleasesUrl/download/v$Version"
    $Asset = "marula-$Version-$Os-$Arch.tar.gz"
    $Sums = "SHA256SUMS-$Os-$Arch"
    $ChecksumList = "$Base/$Sums"

    $Partial = $null
    $Temp = Join-Path ([System.IO.Path]::GetTempPath()) ("marula-download-" + [guid]::NewGuid().ToString('N'))
    New-Item -ItemType Directory -Path $Temp -Force | Out-Null

    try {
        Say "fetching $Asset"
        Get-RemoteFile $ChecksumList (Join-Path $Temp $Sums)
        Get-RemoteFile "$Base/$Sums.sig" (Join-Path $Temp "$Sums.sig")

        # Select-Object -First 1: a name can resolve to more than one file on
        # PATH, and Get-Command then answers with an array whose .Source is not a
        # path any cmdlet accepts.
        $SshKeygen = Get-Command ssh-keygen -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
        if (-not $SshKeygen) {
            Fail 'needs ssh-keygen to verify the release signature; it ships with the Windows OpenSSH client (Settings > Apps > Optional features > OpenSSH Client). Without it a download cannot be trusted'
        }

        # Present is not enough. `-Y verify` arrived in OpenSSH 8.1, and Windows
        # 10 1809 ships 7.7p1, which answers "unknown option -- Y" — which would
        # otherwise surface as a signature-mismatch error blaming the release.
        $KeygenProbe = & $SshKeygen.Source -Y verify 2>&1 | Out-String
        if ($KeygenProbe -match 'unknown option|illegal option|invalid option|unrecognized option') {
            Fail ("the ssh-keygen on PATH is too old: it has no '-Y verify', which OpenSSH 8.1 added. " +
                "Windows 10 1809 ships OpenSSH 7.7p1. Install the current OpenSSH client " +
                "(Settings > Apps > Optional features, or Microsoft's Win32-OpenSSH release), then run this again")
        }
        }

        # A UTF-8 BOM would become part of the first field of the allowed_signers
        # line, and ssh-keygen reads that line strictly, so the bytes are written
        # explicitly without one. Windows PowerShell 5.1's Set-Content adds a BOM
        # when it is asked for UTF-8.
        $AllowedSigners = Join-Path $Temp 'allowed_signers'
        [System.IO.File]::WriteAllText($AllowedSigners, $KeyLine + "`n", (New-Object System.Text.UTF8Encoding($false)))

        Say "verifying the signature over $Sums"
        # ssh-keygen takes the signed message on stdin and checks its exact
        # bytes, so the file is fed to it as bytes. Start-Process cannot be used
        # for that: its -RedirectStandardInput goes through a text writer, which
        # rewrites the line ending and makes every signature fail. The arguments
        # are the file names alone, with the directory set as the working
        # directory, because a joined argument string cannot carry a path with a
        # space in it (a profile directory can have one).
        $Verify = New-Object System.Diagnostics.Process
        $Verify.StartInfo.FileName = $SshKeygen.Source
        $Verify.StartInfo.Arguments = "-Y verify -f allowed_signers -I $KeyId -n file -s $Sums.sig"
        $Verify.StartInfo.WorkingDirectory = $Temp
        $Verify.StartInfo.UseShellExecute = $false
        $Verify.StartInfo.RedirectStandardInput = $true
        $Verify.StartInfo.RedirectStandardOutput = $true
        $Verify.StartInfo.RedirectStandardError = $true
        $null = $Verify.Start()
        $Signed = [System.IO.File]::ReadAllBytes((Join-Path $Temp $Sums))
        $Verify.StandardInput.BaseStream.Write($Signed, 0, $Signed.Length)
        $Verify.StandardInput.BaseStream.Flush()
        $Verify.StandardInput.Close()
        # Both pipes are drained before the wait: a child that fills one would
        # otherwise block for as long as this side waits.
        $null = $Verify.StandardOutput.ReadToEnd()
        $null = $Verify.StandardError.ReadToEnd()
        $Verify.WaitForExit()
        $VerifyStatus = $Verify.ExitCode
        $Verify.Dispose()
        if ($VerifyStatus -ne 0) {
            Fail "the signature does not verify against the release key $KeyFingerprint. Do not install this download"
        }

        Say "checking the archive against the signed list"
        Get-RemoteFile "$Base/$Asset" (Join-Path $Temp $Asset)

        # sha256sum output is `<hex>  <name>` or `<hex> *<name>`. Only the named
        # asset is read: the list may carry lines for files this run did not
        # fetch, and a whole-list check would fail on those.
        function Read-ChecksumLines([string]$Path) {
            foreach ($Line in [System.IO.File]::ReadAllLines($Path)) {
                if ($Line -match '^([0-9a-fA-F]{64}) [ *](.+?)\s*$') {
                    [pscustomobject]@{
                        Hash = $Matches[1].ToLowerInvariant()
                        Name = $Matches[2]
                    }
                }
            }
        }

        $Listed = @(Read-ChecksumLines (Join-Path $Temp $Sums) | Where-Object { $_.Name -eq $Asset })
        if ($Listed.Count -eq 0) { Fail "the signed list does not name $Asset" }
        $Actual = (Get-FileHash -LiteralPath (Join-Path $Temp $Asset) -Algorithm SHA256).Hash.ToLowerInvariant()
        if ($Actual -ne $Listed[0].Hash) {
            Fail "the archive does not match its signed checksum; do not install it"
        }

        # Windows 10 1803 and later ship bsdtar as tar.exe. A plain `tar` is
        # accepted so this script can be exercised with pwsh off Windows; on
        # Windows the first name is the one that resolves.
        $Tar = $null
        foreach ($Candidate in @('tar.exe', 'tar')) {
            $Command = Get-Command $Candidate -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
            if ($Command) { $Tar = $Command.Source; break }
        }
        if (-not $Tar) {
            Fail 'needs tar.exe, which ships with Windows 10 1803 and later. Enable the feature or run this on a build that has it'
        }

        # The bundle is kept: marula.exe, the compose files and the runbooks are
        # in it, and docs/install.md tells an operator to stay there for doctor,
        # upgrades and the optional profiles. It is unpacked beside the download
        # and moved into place only once it is whole.
        $BundleRoot = $null
        if ($env:MARULA_BUNDLE_DIR) {
            $BundleRoot = Resolve-FullPath $env:MARULA_BUNDLE_DIR
        }
        elseif ($env:LOCALAPPDATA) {
            $BundleRoot = Join-Path $env:LOCALAPPDATA 'marula'
        }
        else {
            Fail 'LOCALAPPDATA is not set; set MARULA_BUNDLE_DIR to choose where the unpacked bundle is kept'
        }
        if (-not (Test-Path -LiteralPath $BundleRoot -PathType Container)) {
            New-Item -ItemType Directory -Path $BundleRoot -Force | Out-Null
        }

        Say "unpacking into $BundleRoot"
        $Partial = Join-Path $BundleRoot ('.unpack.' + [guid]::NewGuid().ToString('N'))
        New-Item -ItemType Directory -Path $Partial -Force | Out-Null
        $TarStatus = 1
        try {
            & $Tar -xzf (Join-Path $Temp $Asset) -C $Partial
            $TarStatus = $LASTEXITCODE
        }
        catch {
            $TarStatus = 1
        }
        if ($TarStatus -ne 0) { Fail "could not unpack $Asset (tar exited $TarStatus)" }

        # The archive carries one top-level directory. Move it into place,
        # replacing an older unpack of the same version rather than nesting a
        # second copy.
        $DirectoryName = "marula-$Version-$Os-$Arch"
        $Bundle = Join-Path $BundleRoot $DirectoryName
        if (Test-Path -LiteralPath $Bundle) { Remove-Item -LiteralPath $Bundle -Recurse -Force }
        if (Test-Path -LiteralPath (Join-Path $Partial $DirectoryName) -PathType Container) {
            Move-Item -LiteralPath (Join-Path $Partial $DirectoryName) -Destination $Bundle
            Remove-Item -LiteralPath $Partial -Recurse -Force
        }
        else {
            Move-Item -LiteralPath $Partial -Destination $Bundle
            $Partial = $null
        }

        # Windows has no air-gapped bundle, so the release directory is the
        # directory just unpacked. Both paths are printed because every
        # instruction in docs/install.md that says "the unpacked release
        # directory" means the second one.
        $Release = $Bundle
        Say "kept at $Bundle"
        Say "the release directory is $Release"

        # ---- the handover -------------------------------------------------
        # The bundle's install.sh is POSIX and cannot run here: it looks for an
        # executable named `marula`, and the Windows archive carries
        # marula.exe. What it does is done in this section instead, in its
        # order: check the tree against its own SHA256SUMS, check Docker, then
        # run the CLI's installer with the bundle's own compose file and wizard
        # front end.
        $Executable = Join-Path $Release 'marula.exe'
        $ComposeFile = Join-Path (Join-Path $Release 'deploy') 'docker-compose.yml'
        $UiDirectory = Join-Path (Join-Path (Join-Path $Release 'apps') 'installer') 'dist'
        if (-not (Test-Path -LiteralPath $Executable -PathType Leaf)) {
            Fail "$Executable is missing; unpack the whole archive"
        }
        if (-not (Test-Path -LiteralPath $ComposeFile -PathType Leaf)) {
            Fail "$ComposeFile is missing; unpack the whole archive"
        }
        if (-not (Test-Path -LiteralPath $UiDirectory -PathType Container)) {
            Fail "$UiDirectory is missing; unpack the whole archive"
        }

        # Every file the tree's own list names is recomputed here: this is what
        # `sha256sum -c SHA256SUMS` does on the other platforms, and it is a
        # separate check from the archive's hash (that one proves the download
        # arrived; this one proves the unpack did).
        Say 'checking the bundle against its own checksums'
        $TreeList = Join-Path $Release 'SHA256SUMS'
        if (-not (Test-Path -LiteralPath $TreeList -PathType Leaf)) {
            Fail "$TreeList is missing; unpack the whole archive"
        }
        $Problems = New-Object System.Collections.Generic.List[string]
        foreach ($Entry in Read-ChecksumLines $TreeList) {
            $File = Join-Path $Release $Entry.Name
            if (-not (Test-Path -LiteralPath $File -PathType Leaf)) {
                $Problems.Add("$($Entry.Name) (missing)")
                continue
            }
            $Hash = (Get-FileHash -LiteralPath $File -Algorithm SHA256).Hash.ToLowerInvariant()
            if ($Hash -ne $Entry.Hash) { $Problems.Add("$($Entry.Name) (does not match)") }
        }
        if ($Problems.Count -gt 0) {
            Fail ("the bundle does not match its own SHA256SUMS: " + ($Problems -join '; ') + '. Do not install this download')
        }

        $Docker = Get-Command docker -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1
        if (-not $Docker) {
            Fail 'docker is not on PATH; this bundle needs Docker Desktop (or another Docker with the compose plugin) installed and running'
        }
        $ComposeStatus = 1
        try {
            & $Docker.Source compose version *> $null
            $ComposeStatus = $LASTEXITCODE
        }
        catch {
            $ComposeStatus = 1
        }
        if ($ComposeStatus -ne 0) {
            Fail 'the docker compose plugin is not available; https://docs.docker.com/compose/install/ names the plugin, and Docker Desktop installs it'
        }

        # -Dir and the two fixed flags are this script's business, exactly as in
        # deploy/install.sh: --dir is passed on by this script, and two copies of
        # the same flag would make the CLI refuse the whole command. The compose
        # file and the wizard front end are the bundle's own, and a different one
        # would describe a stack this bundle does not carry.
        $HomeDirectory = if ($HOME) { $HOME } else { $env:USERPROFILE }
        $InstallDirectory = if ($env:MARULA_INSTALL_DIR) { $env:MARULA_INSTALL_DIR } elseif ($HomeDirectory) { Join-Path $HomeDirectory 'marula' } else { $null }
        if (-not $InstallDirectory) { Fail 'neither HOME nor USERPROFILE is set; pass -Dir to name the install directory' }
        $DirectoryGiven = $false
        $InstallerArgs = New-Object System.Collections.Generic.List[string]
        $Index = 0
        while ($Index -lt $PassThrough.Count) {
            $Argument = $PassThrough[$Index]
            if ($Argument -eq '--dir' -or $Argument -eq '-Dir') {
                if ($Index + 1 -ge $PassThrough.Count) { Fail '-Dir needs a value' }
                $InstallDirectory = $PassThrough[$Index + 1]
                $DirectoryGiven = $true
                $Index += 2
            }
            elseif ($Argument -like '--dir=*') {
                $InstallDirectory = $Argument.Substring('--dir='.Length)
                $DirectoryGiven = $true
                $Index += 1
            }
            elseif ($Argument -eq '--compose-file' -or $Argument -like '--compose-file=*' -or
                    $Argument -eq '--ui-dir' -or $Argument -like '--ui-dir=*') {
                Fail (($Argument -split '=')[0] + ' is fixed by the bundle; unpack the archive whole and run this script from it')
            }
            else {
                $InstallerArgs.Add((ConvertTo-InstallerArgument $Argument))
                $Index += 1
            }
        }
        if (-not $InstallDirectory) { Fail '-Dir needs a value' }
        $InstallDirectory = Resolve-FullPath $InstallDirectory
        # With no arguments at all, open the wizard: that is the laptop path, and
        # asking someone to learn a flag before they have seen the product is the
        # friction this script exists to remove. The default images flag below
        # would otherwise make the list non-empty and turn the first-run command
        # into a plan that writes nothing.
        if ($InstallerArgs.Count -eq 0 -and -not $DirectoryGiven) {
            $InstallerArgs.Add('--ui')
        }

        # An archive carries the published digests in release.json and none of
        # the sources, so building is not an option there: every install from a
        # bundle uses the registry unless the caller says otherwise.
        $WantsImages = $false
        foreach ($Argument in $InstallerArgs) {
            if ($Argument -like '--images*') { $WantsImages = $true }
        }
        if ((Test-Path -LiteralPath (Join-Path $Release 'release.json') -PathType Leaf) -and -not $WantsImages) {
            $InstallerArgs.Add('--images')
            $InstallerArgs.Add('registry')
        }

        Say "installing into $InstallDirectory"
        Say 'handing over to the bundle installer'
        # The bundle's own directory is the working directory while the
        # installer runs, because the CLI reads release.json from beside the
        # files it is given. -Dir was resolved above, so a relative one still
        # means the caller's shell and not the bundle.
        Push-Location -LiteralPath $Release
        $Status = 1
        try {
            & $Executable install --compose-file $ComposeFile --ui-dir $UiDirectory --dir $InstallDirectory @InstallerArgs
            $Status = $LASTEXITCODE
        }
        finally {
            Pop-Location
        }
        if ($Status -ne 0) {
            Fail "marula.exe install exited $Status"
        }
    }
    finally {
        if ($Partial -and (Test-Path -LiteralPath $Partial)) {
            Remove-Item -LiteralPath $Partial -Recurse -Force -ErrorAction SilentlyContinue
        }
        if (Test-Path -LiteralPath $Temp) {
            Remove-Item -LiteralPath $Temp -Recurse -Force -ErrorAction SilentlyContinue
        }
    }
}
