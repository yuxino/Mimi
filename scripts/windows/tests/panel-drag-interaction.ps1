# One English panel regression before the existing native language selections.
# The middle blank point is visually calibrated; clipped UIA helper Text bounds
# can extend across it, so pixel and actionable-hit checks remain independent.
function Test-PanelDragInteraction([IntPtr]$Controls,[string]$EvidencePath) {
    if (-not ('MimiCaptionDragDesktop' -as [type])) { Add-Type -Path (Join-Path $PSScriptRoot 'CaptionDragDesktop.cs') }
    $main=[MimiPointerSmoke]::ReadBounds($overlay);$child=[MimiPointerSmoke]::ReadBounds($Controls)
    $scale=$child.Width/280.0
    $metadata=@{rootBefore=@($main.Left,$main.Top,$main.Width,$main.Height);controlBefore=@($child.Left,$child.Top,$child.Width,$child.Height);fullWindows=[MimiCaptionDragDesktop]::FullWindows()}
    function Assert-PanelDragPosition([int]$Offset,[switch]$WaitForPosition) {
        $deadline=[Diagnostics.Stopwatch]::StartNew()
        do {
        Assert-SettingsHidden
        if ((Read-SmokeMarker $SessionStateFile) -ne 'listening') { throw 'Panel dragging changed the active session.' }
        if ([MimiPointerSmoke]::FindVisibleControlWindow($overlay) -ne $Controls) { throw 'Panel dragging replaced or hid its owned control HWND.' }
        $r=[MimiPointerSmoke]::ReadBounds($overlay);$c=[MimiPointerSmoke]::ReadBounds($Controls)
        if ($r.Width -ne $main.Width -or $r.Height -ne $main.Height -or $c.Width -ne $child.Width -or $c.Height -ne $child.Height) {
            throw "The panel did not keep its expanded size while dragging (root=$r control=$c)."
        }
        if ($r.Left -eq $main.Left+$Offset -and $r.Top -eq $main.Top -and $c.Left -eq $child.Left+$Offset -and $c.Top -eq $child.Top) {
            return @{root=@($r.Left,$r.Top,$r.Width,$r.Height);control=@($c.Left,$c.Top,$c.Width,$c.Height)}
        }
        if (-not $WaitForPosition) { break }
        Start-Sleep -Milliseconds 25
        } until ($deadline.ElapsedMilliseconds -ge 2500)
        throw "The panel did not keep its attachment while dragging (offset=$Offset root=$r control=$c)."
    }
    function Get-PanelBlankPoint {
        $frame=[MimiPointerSmoke]::ReadBounds($Controls)
        $x=[int]($frame.Left+160*$scale);$y=[int]($frame.Top+160*$scale)
        if ($frame.Height -lt 220*$scale -or -not [MimiPointerSmoke]::IsCaptionPoint($Controls,$x,$y)) { throw 'The calibrated panel blank point did not hit its native window.' }
        $root=[System.Windows.Automation.AutomationElement]::FromHandle($Controls)
        $types=@('ControlType.Button','ControlType.ComboBox','ControlType.Edit','ControlType.CheckBox','ControlType.RadioButton','ControlType.Slider','ControlType.ScrollBar','ControlType.Hyperlink','ControlType.List','ControlType.ListItem','ControlType.MenuItem')
        foreach ($e in $root.FindAll([System.Windows.Automation.TreeScope]::Descendants,[System.Windows.Automation.Condition]::TrueCondition)) {
            if ($e.Current.IsOffscreen -or $e.Current.ControlType.ProgrammaticName -notin $types) { continue }
            $b=$e.Current.BoundingRectangle
            if (-not $b.IsEmpty -and $x -ge $b.Left-5*$scale -and $x -le $b.Right+5*$scale -and $y -ge $b.Top-5*$scale -and $y -le $b.Bottom+5*$scale) { throw 'The panel blank point overlaps an actionable control.' }
        }
        $hit=[System.Windows.Automation.AutomationElement]::FromPoint([System.Windows.Point]::new($x,$y))
        if ($null -eq $hit -or $hit.Current.ControlType -ne [System.Windows.Automation.ControlType]::Group) { throw 'The panel blank point did not expose its noninteractive group.' }
        $panel=$false
        for ($i=0;$i -lt 24 -and $null -ne $hit;$i++) {
            $isPanel=$hit.Current.AutomationId -ceq 'overlay-control-panel'
            $allowed=@([System.Windows.Automation.ScrollItemPattern]::Pattern)
            if ($isPanel) { $allowed += [System.Windows.Automation.WindowPattern]::Pattern }
            foreach ($pattern in $hit.GetSupportedPatterns()) { if ($pattern -notin $allowed) { throw 'The panel blank point has an actionable ancestor.' } }
            if ($isPanel) { $panel=$true;break }
            $hit=[System.Windows.Automation.TreeWalker]::RawViewWalker.GetParent($hit)
        }
        if (-not $panel) { throw 'The panel blank group is outside the control dialog.' }
        $pixels=[MimiPointerSmoke+PixelPatch]::new($x-4,$y-4,9,9).Read()
        $luma=@($pixels|ForEach-Object{(($_ -band 255)+(($_ -shr 8)-band 255)+(($_ -shr 16)-band 255))/3.0})
        $range=$luma|Measure-Object -Minimum -Maximum
        if ($range.Maximum-$range.Minimum -gt 6) { throw 'The panel blank point contains visible foreground pixels.' }
        return @{X=$x;Y=$y;min=$range.Minimum;max=$range.Maximum}
    }
    try {
        $point=Get-PanelBlankPoint;$metadata.point=$point
        [MimiPointerSmoke]::MoveOverWindow($Controls,$point.X,$point.Y,'Drag the expanded panel from verified middle blank space')
        Start-Sleep -Milliseconds 400
        $metadata.stableBefore=Assert-PanelDragPosition 0
        if (-not [MimiPanelKeyboard]::OwnsFocus($Controls)) { throw 'The open panel did not own focus before dragging.' }
        try {
            [MimiCaptionDragDesktop]::SetLeftButton($true)
            Start-Sleep -Milliseconds 80
            for ($step=1;$step -le 8;$step++) {
                [MimiPointerSmoke]::MoveAndVerify(($point.X+6*$step),$point.Y)
                Start-Sleep -Milliseconds 20
            }
            if (-not [MimiCaptionDragDesktop]::LeftDown()) { throw 'The physical panel drag button was not held.' }
            [MimiPointerSmoke]::AssertCaptionMoveModal($overlay)
            $metadata.held=Assert-PanelDragPosition $(if($metadata.fullWindows){48}else{0}) -WaitForPosition
        } finally { [MimiCaptionDragDesktop]::SetLeftButton($false) }
        Start-Sleep -Milliseconds 400
        $metadata.released=Assert-PanelDragPosition 48
        $null=Wait-PanelLanguageState 'panel-refocus-after-drag' { [MimiPanelKeyboard]::OwnsFocus($Controls) }
        Save-PanelLanguageScreenshot $Controls $EvidencePath 'en-panel-after-drag.png'
        # A short click/drag can finish before the 180ms dismissal and need not
        # produce Moved. It must not leave delayed focus loss armed afterwards.
        $point=Get-PanelBlankPoint
        [MimiPointerSmoke]::MoveOverWindow($Controls,$point.X,$point.Y,'Check a short native panel gesture')
        try { [MimiCaptionDragDesktop]::SetLeftButton($true);Start-Sleep -Milliseconds 15 }
        finally { [MimiCaptionDragDesktop]::SetLeftButton($false) }
        Start-Sleep -Milliseconds 400
        $metadata.shortGesture=Assert-PanelDragPosition 48
        $null=Wait-PanelLanguageState 'panel-refocus-after-short-gesture' { [MimiPanelKeyboard]::OwnsFocus($Controls) }
        [MimiPointerSmoke]::AssertCaptionMoveEnded($overlay)
        $offset=48;$metadata.quickMoves=@()
        foreach ($distance in @(3,48)) {
            $point=Get-PanelBlankPoint
            [MimiPointerSmoke]::MoveOverWindow($Controls,$point.X,$point.Y,'Check a quick native panel move')
            try {
                [MimiCaptionDragDesktop]::SetLeftButton($true)
                Start-Sleep -Milliseconds 8
                [MimiPointerSmoke]::MoveAndVerify(($point.X+$distance),$point.Y)
                Start-Sleep -Milliseconds 7
            } finally { [MimiCaptionDragDesktop]::SetLeftButton($false) }
            Start-Sleep -Milliseconds 400
            $offset+=$distance
            $position=Assert-PanelDragPosition $offset
            $null=Wait-PanelLanguageState 'panel-refocus-after-quick-gesture' { [MimiPanelKeyboard]::OwnsFocus($Controls) }
            [MimiPointerSmoke]::AssertCaptionMoveEnded($overlay)
            if ([MimiCaptionDragDesktop]::LeftDown()) { throw 'A quick panel gesture left the button held.' }
            $metadata.quickMoves+=@{distance=$distance;position=$position}
        }
        $point=Get-PanelBlankPoint
        [MimiPointerSmoke]::ClickOverlay($Controls,$point.X,$point.Y,'An ordinary click in panel blank space')
        Start-Sleep -Milliseconds 400
        $metadata.ordinaryClick=Assert-PanelDragPosition $offset
        $null=Wait-PanelLanguageState 'panel-focus-after-ordinary-click' { [MimiPanelKeyboard]::OwnsFocus($Controls) }
        [MimiPointerSmoke]::AssertCaptionMoveEnded($overlay)
        $metadata.passed=$true
        Write-Host 'Native panel middle drag passed: same HWNDs, +48px, expanded sizes/anchor retained; stationary/quick gestures, ordinary click and native refocus passed. Continuing original language selections.'
    } catch { $metadata.passed=$false;$metadata.failure=[string]$_;throw }
    finally {
        [MimiCaptionDragDesktop]::SetLeftButton($false)
        [IO.File]::WriteAllText((Join-Path $EvidencePath 'en-panel-drag.json'),($metadata|ConvertTo-Json -Depth 6),[Text.UTF8Encoding]::new($false))
    }
}
