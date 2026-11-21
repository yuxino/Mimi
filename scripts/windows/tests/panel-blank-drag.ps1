# One offline, physical blank-panel gesture on the already verified portable.
function Test-PanelBlankDrag([IntPtr]$Controls, $Labels, [string]$EvidencePath) {
    $scope = [System.Windows.Automation.TreeScope]::Descendants
    $type = [System.Windows.Automation.ControlType]
    $metadata = [ordered]@{ scenario = 'panel-blank-horizontal-drag'; locale = $Labels.locale; fullWindows = [MimiCaptionDragDesktop]::FullWindows() }
    [void][IO.Directory]::CreateDirectory($EvidencePath)
    function Wait-BlankPanel([string]$Stage, [scriptblock]$Observe) {
        $timer = [Diagnostics.Stopwatch]::StartNew()
        do {
            Assert-SettingsHidden
            if ((Read-SmokeMarker $SessionStateFile) -ne 'listening') { throw "Session stopped listening at $Stage." }
            try { $result = & $Observe; if ($null -ne $result -and $result -ne $false) { return $result } }
            catch [System.Windows.Automation.ElementNotAvailableException] {}
            Start-Sleep -Milliseconds 25
        } until ($timer.ElapsedMilliseconds -ge 5000)
        throw "Panel blank-drag did not complete $Stage within 5000ms."
    }
    function Read-BlankGeometry {
        $r = [MimiPointerSmoke]::ReadBounds($overlay); $c = [MimiPointerSmoke]::ReadBounds($Controls)
        return @{ root = @($r.Left,$r.Top,$r.Width,$r.Height); control = @($c.Left,$c.Top,$c.Width,$c.Height)
            sameOwnedControl = ([MimiPointerSmoke]::FindVisibleControlWindow($overlay) -eq $Controls)
            pointer = [MimiPointerSmoke]::PointerOwnership($overlay,$Controls); leftDown = [MimiCaptionDragDesktop]::LeftDown() }
    }
    function Assert-BlankGeometry($State, [int]$Delta) {
        if (-not $State.sameOwnedControl) { throw 'The expanded control HWND lost its root ownership.' }
        foreach ($key in @('root','control')) {
            $a = $metadata.before.$key; $b = $State.$key
            if ($b[0] -ne $a[0] + $Delta -or $b[1] -ne $a[1] -or $b[2] -ne $a[2] -or $b[3] -ne $a[3]) {
                throw "Unexpected $key geometry at horizontal offset $Delta (before=$a observed=$b)."
            }
        }
    }
    function Find-BlankField {
        $root = [System.Windows.Automation.AutomationElement]::FromHandle($Controls)
        $byName = [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::NameProperty,$Labels.source)
        foreach ($e in $root.FindAll($scope,$byName)) {
            if ($e.Current.ControlType -in @($type::ComboBox,$type::Button) -and $e.Current.IsEnabled -and -not $e.Current.IsOffscreen) { return $e }
        }
    }
    function Read-BlankPrefs {
        $prefs = (Read-SmokeMarker (Join-Path $FixtureDirectory 'preferences.json')) | ConvertFrom-Json
        if ($prefs.source_language -cne 'en' -or $prefs.target_language -cne 'zh_tw') { throw 'The isolated source/target changed from en/zh_tw.' }
        return @{ source = $prefs.source_language; target = $prefs.target_language }
    }
    # UIA hit must lie in the actual dialog hierarchy, outside every text and
    # actionable rectangle. Pixel flatness independently excludes a glyph.
    function Read-BlankHit([int]$X,[int]$Y) {
        if (-not [MimiPointerSmoke]::IsCaptionPoint($Controls,$X,$Y)) { return }
        $e = [System.Windows.Automation.AutomationElement]::FromPoint([System.Windows.Point]::new($X,$Y))
        if ($null -eq $e -or $e.Current.ControlType -notin @($type::Group,$type::Pane,$type::Document,$type::Custom)) { return }
        $leafType = $e.Current.ControlType.ProgrammaticName
        for ($i=0;$i -lt 24 -and $null -ne $e;$i++) {
            if ($e.Current.ControlType -in $forbidden) { return }
            if ($e.Current.AutomationId -ceq 'overlay-control-panel') { return @{ leafType=$leafType; dialogAncestor=$true; controlHwndHit=$true } }
            $e = [System.Windows.Automation.TreeWalker]::RawViewWalker.GetParent($e)
        }
    }
    $forbidden = @($type::Text,$type::Button,$type::Edit,$type::ComboBox,$type::CheckBox,$type::RadioButton,
        $type::Slider,$type::ScrollBar,$type::Hyperlink,$type::List,$type::ListItem,$type::MenuItem,$type::Menu,
        $type::Tab,$type::TabItem,$type::Spinner,$type::Tree,$type::TreeItem)
    try {
        $compact = Use-OverlayControl $Labels.openControls 'listening' 'Locate compact language entry' -Surface $Controls -NameSuffix -BoundsOnly
        Move-OverControlBounds $Controls $compact 'Reveal the compact language entry'
        Use-OverlayControl $Labels.openControls 'listening' 'Open the language panel once' -Surface $Controls -NameSuffix
        $field = Wait-BlankPanel 'the expanded language field' { Find-BlankField }
        $metadata.initialCodes = Read-BlankPrefs
        $closed = (Get-PanelPickerValueEvidence $field 'source').value
        $root = [System.Windows.Automation.AutomationElement]::FromHandle($Controls)
        $byId = [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::AutomationIdProperty,'overlay-control-panel')
        $dialog = $root.FindFirst($scope,$byId)
        if ($null -eq $dialog) { throw 'No actual panel dialog is exposed for blank-area hit validation.' }
        $box = $dialog.Current.BoundingRectangle
        $scale = [MimiPanelCrop]::GetDpiForWindow($Controls)/96.0
        $blocked = @($dialog.FindAll($scope,[System.Windows.Automation.Condition]::TrueCondition) | Where-Object {
            -not $_.Current.IsOffscreen -and $_.Current.ControlType -in $forbidden
        } | ForEach-Object { $_.Current.BoundingRectangle } | Where-Object { -not $_.IsEmpty -and $_.Width -gt 0 -and $_.Height -gt 0 })
        $candidates = @()
        for ($y=[int]($box.Top+40*$scale);$y -lt $box.Bottom-40*$scale;$y+=8) {
            for ($x=[int]($box.Left+16*$scale);$x -lt $box.Right-16*$scale;$x+=8) {
                $occupied=$false
                foreach ($b in $blocked) { if ($x -ge $b.Left-5*$scale -and $x -le $b.Right+5*$scale -and $y -ge $b.Top-5*$scale -and $y -le $b.Bottom+5*$scale) { $occupied=$true;break } }
                if (-not $occupied) { $candidates += @{X=$x;Y=$y;Score=[Math]::Pow($x-($box.Left+$box.Width/2),2)+[Math]::Pow($y-($box.Top+$box.Height/2),2)} }
            }
        }
        $point=$null
        foreach ($candidate in @($candidates | Sort-Object Score)) {
            $hit=Read-BlankHit $candidate.X $candidate.Y
            if ($null -eq $hit) { continue }
            $patch=[MimiPointerSmoke+PixelPatch]::new($candidate.X-4,$candidate.Y-4,9,9).Read()
            $luma=@($patch | ForEach-Object { (($_ -band 255)+(($_ -shr 8)-band 255)+(($_ -shr 16)-band 255))/3.0 })
            $range=($luma|Measure-Object -Minimum -Maximum)
            if ($range.Maximum-$range.Minimum -gt 6) { continue }
            $point=$candidate;$metadata.hit=$hit;$metadata.patch=@{width=9;height=9;min=$range.Minimum;max=$range.Maximum};break
        }
        if ($null -eq $point) { throw 'No verifiable noninteractive, nontext, nonedge blank point was found.' }
        [MimiPointerSmoke]::MoveOverWindow($Controls,$point.X,$point.Y,'Hold verified panel blank space')
        $metadata.point=@($point.X,$point.Y);$metadata.before=Read-BlankGeometry
        Start-Sleep -Milliseconds 400
        $metadata.stableBefore=Read-BlankGeometry
        Assert-BlankGeometry $metadata.stableBefore 0
        if ($null -eq (Read-BlankHit $point.X $point.Y)) { throw 'Blank-area physical/UIA hit changed before the press.' }
        Save-PanelLanguageScreenshot $Controls $EvidencePath 'panel-before.png'
        $heldFailure=$null
        try {
            [MimiCaptionDragDesktop]::SetLeftButton($true)
            Start-Sleep -Milliseconds 80
            for ($step=1;$step -le 8;$step++) {
                [MimiPointerSmoke]::MoveAndVerify(($point.X+6*$step),$point.Y)
                Start-Sleep -Milliseconds 20
            }
            $metadata.held=Read-BlankGeometry
            if (-not $metadata.held.leftDown) { throw 'Physical left button was not held after the gesture.' }
            [MimiPointerSmoke]::AssertCaptionMoveModal($overlay)
        } catch { $heldFailure=$_ }
        finally { [MimiCaptionDragDesktop]::SetLeftButton($false) }
        $metadata.released=Read-BlankGeometry
        Start-Sleep -Milliseconds 400
        $metadata.settled=Read-BlankGeometry
        Assert-SettingsHidden
        if ((Read-SmokeMarker $SessionStateFile) -ne 'listening') { throw 'The panel gesture changed session state.' }
        if ($metadata.settled.leftDown) { throw 'The panel gesture left the mouse button held.' }
        Assert-BlankGeometry $metadata.settled 48
        if ($null -ne $heldFailure) { throw $heldFailure }
        Save-PanelLanguageScreenshot $Controls $EvidencePath 'panel-after.png'
        $field=Wait-BlankPanel 'the moved source language trigger' { Find-BlankField }
        if ((Get-PanelPickerValueEvidence $field 'moved source').value -cne $closed) { throw 'The blank gesture changed the source field value.' }
        $b=$field.Current.BoundingRectangle
        [MimiPointerSmoke]::ClickOverlay($Controls,[int]($b.Left+$b.Width/2),[int]($b.Top+$b.Height/2),'Open the source language menu once after panel drag')
        $selected=Wait-BlankPanel 'the real source menu with one selected original value' {
            $r=[System.Windows.Automation.AutomationElement]::FromHandle($Controls)
            $condition=[System.Windows.Automation.AndCondition]::new([System.Windows.Automation.Condition[]]@(
                [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::NameProperty,$Labels.source),
                [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::ControlTypeProperty,$type::List)))
            $list=$r.FindFirst($scope,$condition)
            if ($null -eq $list -or $list.Current.IsOffscreen) { return }
            $items=$list.FindAll($scope,[System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::ControlTypeProperty,$type::ListItem))
            $chosen=@()
            foreach ($item in $items) {
                $pattern=$null
                if ($item.TryGetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern,[ref]$pattern) -and $pattern.Current.IsSelected) { $chosen+= $item }
            }
            if ($items.Count -lt 2 -or $chosen.Count -ne 1) { return }
            if ($chosen[0].Current.Name -cne $closed) { throw 'The actual selected menu option differs from the original closed value.' }
            $metadata.menu=@{optionCount=$items.Count;originalSelected=$true;nativeFocus=[MimiPanelKeyboard]::OwnsFocus($Controls)}
            return $chosen[0]
        }
        if (-not $selected.Current.IsEnabled -or $selected.Current.IsOffscreen) { throw 'The original language option is not clickable.' }
        Save-PanelLanguageScreenshot $Controls $EvidencePath 'panel-menu.png'
        $b=$selected.Current.BoundingRectangle
        if ($b.IsEmpty -or $b.Width -le 0 -or $b.Height -le 0) { throw 'The selected language option has no actual target.' }
        [MimiPointerSmoke]::ClickOverlay($Controls,[int]($b.Left+$b.Width/2),[int]($b.Top+$b.Height/2),'Click the already selected language option once')
        $null=Wait-BlankPanel 'language menu closure without a value change' {
            $f=Find-BlankField
            if ($null -eq $f -or -not $f.Current.HasKeyboardFocus) { return }
            $r=[System.Windows.Automation.AutomationElement]::FromHandle($Controls)
            foreach ($list in $r.FindAll($scope,[System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::ControlTypeProperty,$type::List))) {
                if (-not $list.Current.IsOffscreen) { return }
            }
            if ((Get-PanelPickerValueEvidence $f 'final source').value -cne $closed) { throw 'The final source language field changed.' }
            return $true
        }
        $metadata.finalCodes=Read-BlankPrefs
        $metadata.final=Read-BlankGeometry
        Assert-BlankGeometry $metadata.final 48
        $metadata.passed=$true
        Write-Host "Panel blank drag passed: horizontal=48 vertical=0; same root/child HWNDs and sizes; real selected menu option clicked; source=en target=zh_tw; Settings hidden/listening."
    } catch {
        $metadata.passed=$false;$metadata.failure=[string]$_
        try { Save-PanelLanguageScreenshot $Controls $EvidencePath 'panel-failed.png' } catch { Write-Warning 'Guarded synthetic panel failure screenshot unavailable.' }
        throw
    } finally {
        [MimiCaptionDragDesktop]::SetLeftButton($false)
        [IO.File]::WriteAllText((Join-Path $EvidencePath 'panel-blank-drag.json'),($metadata|ConvertTo-Json -Depth 8),[Text.UTF8Encoding]::new($false))
    }
}
