# One offline, physical blank-panel gesture on the already verified portable.
function Test-PanelBlankDrag([IntPtr]$Controls, $Labels, [string]$EvidencePath) {
    $scope = [System.Windows.Automation.TreeScope]::Descendants
    $type = [System.Windows.Automation.ControlType]
    $observation=$null
    . (Join-Path $PSScriptRoot 'panel-drag-observation.ps1')
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
    function Trace-BlankPoint([int]$X,[int]$Y) {
        $trace = @{ point=@($X,$Y); controlHwndHit=[MimiPointerSmoke]::IsCaptionPoint($Controls,$X,$Y); path=@() }
        $e=[System.Windows.Automation.AutomationElement]::FromPoint([System.Windows.Point]::new($X,$Y))
        for ($i=0;$i -lt 24 -and $null -ne $e;$i++) {
            $b=$e.Current.BoundingRectangle
            $trace.path += @{ type=$e.Current.ControlType.ProgrammaticName; hasName=(-not [string]::IsNullOrEmpty($e.Current.Name))
                panelId=($e.Current.AutomationId -ceq 'overlay-control-panel'); bounds=@($b.Left,$b.Top,$b.Width,$b.Height)
                patterns=@($e.GetSupportedPatterns() | ForEach-Object { $_.ProgrammaticName }) }
            if ($e.Current.AutomationId -ceq 'overlay-control-panel') { break }
            $e=[System.Windows.Automation.TreeWalker]::RawViewWalker.GetParent($e)
        }
        if ($trace.controlHwndHit) {
            $pixels=[MimiPointerSmoke+PixelPatch]::new($X-4,$Y-4,9,9).Read()
            $values=@($pixels | ForEach-Object { (($_ -band 255)+(($_ -shr 8)-band 255)+(($_ -shr 16)-band 255))/3.0 })
            $range=$values | Measure-Object -Minimum -Maximum
            $trace.patch=@{width=9;height=9;pixels=$pixels;min=$range.Minimum;max=$range.Maximum}
        }
        return $trace
    }
    function Reject-BlankHit([string]$Reason,[int]$X,[int]$Y) {
        if (-not $metadata.hitRejectCounts.Contains($Reason)) { $metadata.hitRejectCounts[$Reason]=0 }
        $metadata.hitRejectCounts[$Reason]=$metadata.hitRejectCounts[$Reason]+1
        if (-not $metadata.firstHitRejection.Contains($Reason)) { $metadata.firstHitRejection[$Reason]=Trace-BlankPoint $X $Y }
    }
    # UIA hit must lie in the actual dialog hierarchy, outside every text and
    # actionable rectangle. Pixel flatness independently excludes a glyph.
    function Read-BlankHit([int]$X,[int]$Y) {
        if (-not [MimiPointerSmoke]::IsCaptionPoint($Controls,$X,$Y)) { Reject-BlankHit 'nativeHwnd' $X $Y; return }
        $e = [System.Windows.Automation.AutomationElement]::FromPoint([System.Windows.Point]::new($X,$Y))
        if ($null -eq $e) { Reject-BlankHit 'nullFromPoint' $X $Y; return }
        if ($e.Current.ControlType -notin @($type::Group,$type::Pane,$type::Document,$type::Custom)) { Reject-BlankHit 'leafType' $X $Y; return }
        $leafType = $e.Current.ControlType.ProgrammaticName
        for ($i=0;$i -lt 24 -and $null -ne $e;$i++) {
            if ($e.Current.ControlType -in $forbidden) { Reject-BlankHit 'ancestorType' $X $Y; return }
            if ($e.Current.AutomationId -ceq 'overlay-control-panel') { return @{ leafType=$leafType; dialogAncestor=$true; controlHwndHit=$true } }
            $e = [System.Windows.Automation.TreeWalker]::RawViewWalker.GetParent($e)
        }
        Reject-BlankHit 'noPanelAncestor' $X $Y
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
        $metadata.nativeStart=Read-BlankGeometry
        $shot=Save-PanelLanguageScreenshot $Controls $EvidencePath 'panel-map.png' -PassThru
        $metadata.panelMap=@{ dpi=$shot.dpi; dialog=@($box.Left,$box.Top,$box.Width,$box.Height)
            crop=@($shot.crop.Left,$shot.crop.Top,$shot.crop.Width,$shot.crop.Height)
            rootHwnd=$overlay.ToInt64(); controlHwnd=$Controls.ToInt64() }
        $metadata.hitRejectCounts=[ordered]@{}; $metadata.firstHitRejection=[ordered]@{}
        $metadata.pixelRejected=0; $metadata.scanPoints=0; $metadata.blockedPoints=0; $metadata.blockedRects=@(); $blockedElements=@()
        $blocked=@()
        foreach ($element in $dialog.FindAll($scope,[System.Windows.Automation.Condition]::TrueCondition)) {
            if ($element.Current.IsOffscreen -or $element.Current.ControlType -notin $forbidden) { continue }
            $b=$element.Current.BoundingRectangle
            if ($b.IsEmpty -or $b.Width -le 0 -or $b.Height -le 0) { continue }
            $blocked += $b; $blockedElements += $element
            $metadata.blockedRects += @{type=$element.Current.ControlType.ProgrammaticName;bounds=@($b.Left,$b.Top,$b.Width,$b.Height);area=$b.Width*$b.Height;exclusions=0}
        }
        # Explicit observation points correspond to visually flat gaps in the
        # prior 280x403 native crop. They are never substituted as gesture points.
        $metadata.referencePoints=@()
        foreach ($relative in @(@(160,160),@(180,184),@(180,316))) {
            $x=[int]($box.Left+$relative[0]*$scale); $y=[int]($box.Top+$relative[1]*$scale)
            if ($x -gt $box.Left+5 -and $x -lt $box.Right-5 -and $y -gt $box.Top+5 -and $y -lt $box.Bottom-5) {
                $metadata.referencePoints += Trace-BlankPoint $x $y
            }
        }
        $candidates = @()
        for ($y=[int]($box.Top+40*$scale);$y -lt $box.Bottom-40*$scale;$y+=8) {
            for ($x=[int]($box.Left+16*$scale);$x -lt $box.Right-16*$scale;$x+=8) {
                $occupied=$false; $metadata.scanPoints=$metadata.scanPoints+1
                for ($i=0;$i -lt $blocked.Count;$i++) {
                    $b=$blocked[$i]
                    if ($x -ge $b.Left-5*$scale -and $x -le $b.Right+5*$scale -and $y -ge $b.Top-5*$scale -and $y -le $b.Bottom+5*$scale) {
                        $occupied=$true; $metadata.blockedPoints=$metadata.blockedPoints+1
                        $metadata.blockedRects[$i].exclusions=$metadata.blockedRects[$i].exclusions+1; break
                    }
                }
                if (-not $occupied) { $candidates += @{X=$x;Y=$y;Score=[Math]::Pow($x-($box.Left+$box.Width/2),2)+[Math]::Pow($y-($box.Top+$box.Height/2),2)} }
            }
        }
        $metadata.afterRectangleExclusion=$candidates.Count
        $point=$null
        foreach ($candidate in @($candidates | Sort-Object Score)) {
            $hit=Read-BlankHit $candidate.X $candidate.Y
            if ($null -eq $hit) { continue }
            $patch=[MimiPointerSmoke+PixelPatch]::new($candidate.X-4,$candidate.Y-4,9,9).Read()
            $luma=@($patch | ForEach-Object { (($_ -band 255)+(($_ -shr 8)-band 255)+(($_ -shr 16)-band 255))/3.0 })
            $range=($luma|Measure-Object -Minimum -Maximum)
            if ($range.Maximum-$range.Minimum -gt 6) { $metadata.pixelRejected=$metadata.pixelRejected+1; continue }
            $point=$candidate;$metadata.hit=$hit;$metadata.patch=@{width=9;height=9;min=$range.Minimum;max=$range.Maximum};break
        }
        $metadata.originalStrictPointAvailable=($null -ne $point)
        # A separately named visual experiment. Keep the conservative strict
        # result and all text-owner diagnostics; neither guesses DOM identity.
        $metadata.scenario='visible-middle-blank-horizontal-drag'
        $expected=Get-Content -LiteralPath (Join-Path $PSScriptRoot 'panel-blank-description.json') -Raw -Encoding UTF8 | ConvertFrom-Json
        $point=@{X=[int]($box.Left+160*$scale);Y=[int]($box.Top+160*$scale)}
        if ($point.X -le $box.Left+16*$scale -or $point.X -ge $box.Right-16*$scale -or
            $point.Y -le $box.Top+40*$scale -or $point.Y -ge $box.Bottom-40*$scale) { throw 'The middle blank point is near an edge.' }
        $textOverlaps=@()
        for ($i=0;$i -lt $blocked.Count;$i++) {
            $b=$blocked[$i]
            if ($point.X -ge $b.Left-5*$scale -and $point.X -le $b.Right+5*$scale -and
                $point.Y -ge $b.Top-5*$scale -and $point.Y -le $b.Bottom+5*$scale) {
                $element=$blockedElements[$i]
                if ($element.Current.ControlType -ne $type::Text) { throw 'The visual blank point overlaps an actionable control boundary.' }
                $textOverlaps += @{bounds=@($b.Left,$b.Top,$b.Width,$b.Height);name=$element.Current.Name}
            }
        }
        $hit=Read-BlankHit $point.X $point.Y
        if ($null -eq $hit -or $hit.leafType -cne 'ControlType.Group') { throw 'The middle point did not hit the calibrated noninteractive native group.' }
        $element=[System.Windows.Automation.AutomationElement]::FromPoint([System.Windows.Point]::new($point.X,$point.Y))
        for ($i=0;$i -lt 24 -and $null -ne $element;$i++) {
            $isPanel=$element.Equals($dialog)
            $allowed=@([System.Windows.Automation.ScrollItemPattern]::Pattern)
            if ($isPanel) { $allowed += [System.Windows.Automation.WindowPattern]::Pattern }
            foreach ($pattern in $element.GetSupportedPatterns()) { if ($pattern -notin $allowed) { throw 'The middle point has an actionable UIA pattern.' } }
            if ($isPanel) { break }
            $element=[System.Windows.Automation.TreeWalker]::RawViewWalker.GetParent($element)
        }
        if ($null -eq $element -or -not $element.Equals($dialog)) { throw 'The middle group does not belong to the exact same dialog.' }
        $metadata.visibleBlank=@{local=@(160,160);nativeHit=Trace-BlankPoint $point.X $point.Y;textOverlaps=$textOverlaps
            offlineOwnerRect=$expected.ownerRect;offlineClipPath=$expected.clipPath;offlineReference=$expected.sourceReference;associationProperties=@()}
        # This is built-in UI copy in a fresh offline fixture, never captions,
        # provider payloads or the user's configuration. Record it before input.
        $condition=[System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::NameProperty,$expected.helpLabel)
        $owners=@($root.FindAll($scope,$condition) | Where-Object { $_.Current.ControlType -eq $type::Button -and -not $_.Current.IsOffscreen })
        $metadata.visibleBlank.publicOwnerCount=$owners.Count
        function Normalize-BlankDescription([string]$Text) { return [regex]::Replace($Text,'\s+',' ').Trim() }
        $metadata.visibleBlank.completeOfflineDescriptionMatch=($textOverlaps.Count -eq 1 -and
            (Normalize-BlankDescription $textOverlaps[0].name) -ceq (Normalize-BlankDescription $expected.text))
        if ($owners.Count -eq 1) {
            foreach ($property in $owners[0].GetSupportedProperties()) {
                if ($property.ProgrammaticName -notmatch '(HelpText|FullDescription|DescribedBy)Property$') { continue }
                $value=$owners[0].GetCurrentPropertyValue($property,$true)
                $entry=@{property=$property.ProgrammaticName;available=($value -ne [System.Windows.Automation.AutomationElement]::NotSupported)}
                if ($value -is [string]) { $entry.value=$value }
                elseif ($value -is [System.Windows.Automation.AutomationElement[]]) { $entry.descriptions=@($value | ForEach-Object {$_.Current.Name}) }
                $metadata.visibleBlank.associationProperties += $entry
            }
        }
        $patch=[MimiPointerSmoke+PixelPatch]::new($point.X-4,$point.Y-4,9,9).Read()
        $luma=@($patch | ForEach-Object { (($_ -band 255)+(($_ -shr 8)-band 255)+(($_ -shr 16)-band 255))/3.0 })
        $range=$luma | Measure-Object -Minimum -Maximum
        if ($range.Maximum-$range.Minimum -gt 6) { throw 'The middle point is not visibly flat under the original pixel threshold.' }
        $metadata.patch=@{width=9;height=9;min=$range.Minimum;max=$range.Maximum}
        $metadata.visibleBlank.nativeForegroundOwned=[MimiPanelKeyboard]::OwnsFocus($Controls)
        if (-not $metadata.visibleBlank.nativeForegroundOwned) { throw 'The isolated control window does not own native foreground and focus.' }
        $observation=Start-PanelDragObservation $overlay $Controls $EvidencePath
        $observation.Observer.Mark('move-to-blank')
        [MimiPointerSmoke]::MoveOverWindow($Controls,$point.X,$point.Y,'Hold verified panel blank space')
        $metadata.point=@($point.X,$point.Y);$metadata.before=Read-BlankGeometry
        Start-Sleep -Milliseconds 400
        $metadata.stableBefore=Read-BlankGeometry
        Assert-BlankGeometry $metadata.stableBefore 0
        if ($null -eq (Read-BlankHit $point.X $point.Y)) { throw 'Blank-area physical/UIA hit changed before the press.' }
        Save-PanelLanguageScreenshot $Controls $EvidencePath 'panel-before.png'
        $observation.Observer.Mark('before-press')
        if (-not [MimiPanelKeyboard]::OwnsFocus($Controls)) { throw 'The control lost native foreground before input.' }
        $heldFailure=$null
        try {
            [MimiCaptionDragDesktop]::SetLeftButton($true)
            $observation.Observer.Mark('pressed')
            Start-Sleep -Milliseconds 80
            for ($step=1;$step -le 8;$step++) {
                [MimiPointerSmoke]::MoveAndVerify(($point.X+6*$step),$point.Y)
                Start-Sleep -Milliseconds 20
            }
            $metadata.held=Read-BlankGeometry
            $observation.Observer.Mark('held')
            if (-not $metadata.held.leftDown) { throw 'Physical left button was not held after the gesture.' }
            [MimiPointerSmoke]::AssertCaptionMoveModal($overlay)
            $metadata.rootMoveModalVerified=$true
        } catch { $heldFailure=$_;$metadata.heldFailure=[string]$_ }
        finally { $observation.Observer.Mark('before-release');[MimiCaptionDragDesktop]::SetLeftButton($false);$observation.Observer.Mark('released') }
        $metadata.released=Read-BlankGeometry
        Start-Sleep -Milliseconds 400
        $metadata.settled=Read-BlankGeometry
        $observation.Observer.Mark('settled')
        Assert-SettingsHidden
        if ((Read-SmokeMarker $SessionStateFile) -ne 'listening') { throw 'The panel gesture changed session state.' }
        if ($metadata.settled.leftDown) { throw 'The panel gesture left the mouse button held.' }
        Assert-BlankGeometry $metadata.settled 48
        if ($null -ne $heldFailure) { throw $heldFailure }
        Save-PanelLanguageScreenshot $Controls $EvidencePath 'panel-after.png'
        $null=Wait-BlankPanel 'long-gesture native focus recovery' { [MimiPanelKeyboard]::OwnsFocus($Controls) }
        $observation.Observer.Mark('short-press')
        $frame=[MimiPointerSmoke]::ReadBounds($Controls)
        $shortX=[int]($frame.Left+160*$scale);$shortY=[int]($frame.Top+160*$scale)
        [MimiPointerSmoke]::MoveOverWindow($Controls,$shortX,$shortY,'Check a short stationary panel gesture')
        try{[MimiCaptionDragDesktop]::SetLeftButton($true);Start-Sleep -Milliseconds 15}
        finally{[MimiCaptionDragDesktop]::SetLeftButton($false)}
        $observation.Observer.Mark('short-released')
        Start-Sleep -Milliseconds 400
        $observation.Observer.Mark('short-settled')
        $metadata.shortGesture=Read-BlankGeometry;Assert-BlankGeometry $metadata.shortGesture 48
        $metadata.diagnosticRelease=$observation.Observer.ObserveReleaseSignal()
        $null=Wait-BlankPanel 'short-gesture native focus recovery' { [MimiPanelKeyboard]::OwnsFocus($Controls) }
        $metadata.afterDiagnosticRelease=Read-BlankGeometry;Assert-BlankGeometry $metadata.afterDiagnosticRelease 48
        $metadata.releaseSignalVerified=$true
        $metadata.quickMoveObservations=@()
        foreach($distance in @(3,48)){
            $start=Read-BlankGeometry;$c=[MimiPointerSmoke]::ReadBounds($Controls)
            $x=[int]($c.Left+160*$scale);$y=[int]($c.Top+160*$scale)
            [MimiPointerSmoke]::MoveOverWindow($Controls,$x,$y,'Observe a bounded quick owned move')
            $observation.Observer.Mark(('quick-'+$distance+'-before'))
            try{
                [MimiCaptionDragDesktop]::SetLeftButton($true)
                Start-Sleep -Milliseconds 8
                [MimiPointerSmoke]::MoveAndVerify(($x+$distance),$y)
                Start-Sleep -Milliseconds 7
            }finally{[MimiCaptionDragDesktop]::SetLeftButton($false)}
            $observation.Observer.Mark(('quick-'+$distance+'-released'))
            Start-Sleep -Milliseconds 400
            $first=Read-BlankGeometry
            $forwarded=$observation.Observer.ObserveReleaseSignal()
            $null=Wait-BlankPanel 'quick-gesture native completion' { [MimiPanelKeyboard]::OwnsFocus($Controls) }
            $last=Read-BlankGeometry
            $metadata.quickMoveObservations+=@{distance=$distance;start=$start;first=$first;forwarded=$forwarded;completed=$last}
            $observation.Observer.Mark(('quick-'+$distance+'-completed'))
        }
        throw 'DIAGNOSTIC ONLY: real short/quick gestures with observed nonclient release. Inspect original geometry and native trace; no product pass.'
        Save-PanelLanguageScreenshot $Controls $EvidencePath 'panel-after-short.png'
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
        $observation.Observer.Mark('external-click-away')
        $outside=$observation.Observer.EnableExternalClickAway()
        [MimiPointerSmoke]::ClickOverlay($observation.Observer.BackgroundWindow,$outside.X,$outside.Y,'Click the controlled external neutral window')
        $null=Wait-BlankPanel 'true external focus closes the expanded panel' {
            $c=[MimiPointerSmoke]::ReadBounds($Controls)
            if($c.Height -eq [int](30*$scale) -and -not [MimiPanelKeyboard]::OwnsFocus($Controls)){return $true}
        }
        $metadata.externalClickAway=Read-BlankGeometry
        $observation.Observer.Mark('external-focus-dismissed')
        Start-Sleep -Milliseconds 400
        # Explicit reopen and collapse stay ordinary physical actions.
        $capsule=Use-OverlayControl $Labels.openControls 'listening' 'Locate the compact entry after click-away' -Surface $Controls -NameSuffix -BoundsOnly
        Move-OverControlBounds $Controls $capsule 'Reveal the compact entry after click-away'
        Start-Sleep -Milliseconds 240
        Use-OverlayControl $Labels.openControls 'listening' 'Reopen controls after external click-away' -Surface $Controls -NameSuffix
        $null=Wait-BlankPanel 'explicit reopen' { $c=[MimiPointerSmoke]::ReadBounds($Controls);if($c.Width -eq $metadata.before.control[2] -and $c.Height -eq $metadata.before.control[3]){$true} }
        Use-OverlayControl 'Collapse subtitle controls' 'listening' 'Explicitly collapse the control panel' -Surface $Controls -NameSuffix
        $null=Wait-BlankPanel 'explicit collapse remains effective' { $c=[MimiPointerSmoke]::ReadBounds($Controls);if($c.Height -eq [int](30*$scale)){$true} }
        $metadata.explicitCollapse=Read-BlankGeometry
        $observation.Observer.Mark('lock-check')
        $lock=Use-OverlayControl 'Lock Subtitle Position' 'listening' 'Locate position lock in the root toolbar' -BoundsOnly
        Move-OverControlBounds $overlay $lock 'Reveal position lock in the root toolbar'
        Start-Sleep -Milliseconds 240
        Use-OverlayControl 'Lock Subtitle Position' 'listening' 'Lock subtitle position from its real toolbar button'
        $null=Wait-BlankPanel 'the saved position lock' { $p=(Read-SmokeMarker (Join-Path $FixtureDirectory 'preferences.json'))|ConvertFrom-Json;if($p.overlay_locked -eq $true){$true} }
        $r=[MimiPointerSmoke]::ReadBounds($overlay);$bodyX=[int]($r.Left+$r.Width/2);$bodyY=[int]($r.Top+$r.Height/2)
        [MimiPointerSmoke]::MoveAndVerify($bodyX,$bodyY)
        if($observation.Observer.HitSurface($bodyX,$bodyY) -cne 'qa-background'){throw 'The locked root did not pass its body hit through to the controlled neutral background.'}
        try{
            [MimiCaptionDragDesktop]::SetLeftButton($true)
            [MimiPointerSmoke]::MoveAndVerify(($bodyX+48),$bodyY)
        }finally{[MimiCaptionDragDesktop]::SetLeftButton($false)}
        Start-Sleep -Milliseconds 400
        $metadata.locked=Read-BlankGeometry
        $r=[MimiPointerSmoke]::ReadBounds($overlay)
        if($r.Left -ne $metadata.before.root[0]+48 -or $r.Top -ne $metadata.before.root[1] -or $r.Width -ne $metadata.before.root[2] -or $r.Height -ne $metadata.before.root[3]){throw 'Position lock allowed a root move or resize.'}
        $unlock=Use-OverlayControl 'Unlock Subtitle Position' 'listening' 'Locate the independent unlock entry' -Surface $Controls -BoundsOnly
        Move-OverControlBounds $Controls $unlock 'Reveal the independent unlock entry'
        Start-Sleep -Milliseconds 240
        Use-OverlayControl 'Unlock Subtitle Position' 'listening' 'Unlock through the independent capsule once' -Surface $Controls
        $null=Wait-BlankPanel 'the saved unlock' { $p=(Read-SmokeMarker (Join-Path $FixtureDirectory 'preferences.json'))|ConvertFrom-Json;if($p.overlay_locked -eq $false){$true} }
        $null=Wait-BlankPanel 'unlocked root interaction' { if([MimiPointerSmoke]::IsCaptionPoint($overlay,$bodyX,$bodyY)){$true} }
        $metadata.unlocked=Read-BlankGeometry
        $metadata.finalCodes=Read-BlankPrefs
        Assert-SettingsHidden
        $observation.Observer.Mark('all-native-cases-complete')
        Start-Sleep -Milliseconds 500
        $metadata.passed=$true
        Write-Host "Visible middle blank observation passed: horizontal=48 vertical=0; same root/child HWNDs and sizes; real selected menu option clicked; source=en target=zh_tw; Settings hidden/listening."
    } catch {
        $metadata.passed=$false;$metadata.failure=[string]$_
        if($observation){$observation.Observer.Mark('assertion-failure');Start-Sleep -Milliseconds 750}
        try {
            if($observation){
                $frame=[MimiPointerSmoke]::ReadBounds($Controls);$region=$observation.Observer.Region
                if($frame.Left -lt $region.Left -or $frame.Top -lt $region.Top -or $frame.Right -gt $region.Right -or $frame.Bottom -gt $region.Bottom){throw 'Failure crop is outside the controlled neutral background.'}
            }
            Save-PanelLanguageScreenshot $Controls $EvidencePath 'panel-failed.png'
        } catch { Write-Warning 'Guarded synthetic panel failure screenshot unavailable.' }
        throw
    } finally {
        [MimiCaptionDragDesktop]::SetLeftButton($false)
        if($observation){try{Stop-PanelDragObservation $observation}catch{$metadata.observationFailure=[string]$_;Write-Warning 'Native recording/trace finalization failed; original interaction result preserved.'}}
        [IO.File]::WriteAllText((Join-Path $EvidencePath 'panel-blank-drag.json'),($metadata|ConvertTo-Json -Depth 12),[Text.UTF8Encoding]::new($false))
    }
}
