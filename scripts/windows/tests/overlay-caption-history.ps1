# Real input against the isolated 01..20 synthetic-history fixture. ScrollPattern
# is an observer only: no UIA/programmatic scroll or focus activation is used.
function Get-CaptionHistoryRows($Context) {
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($Context.Window)
    $candidates = @{}
    foreach ($element in $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, [System.Windows.Automation.Condition]::TrueCondition)) {
        $name = [string]$element.Current.Name
        $matches = [regex]::Matches($name, $Context.CaptionPattern)
        # Ignore aggregate accessible parents containing several caption lanes.
        if ($matches.Count -ne 1 -or $matches[0].Index -ne 0) { continue }
        $match = $matches[0]
        $number = [int]$match.Groups['number'].Value
        if ($number -lt 1 -or $number -gt 20) { continue }
        $kind = if ($match.Groups['kind'].Value -ceq 'Original') { 'source' } else { 'translation' }
        $key = '{0}{1:00}' -f $kind, $number
        $box = $element.Current.BoundingRectangle
        $area = if ($box.IsEmpty -or $box.Width -le 0 -or $box.Height -le 0) { 0 } else { $box.Width * $box.Height }
        # Offscreen names can prove delivery even when WebView2 clips their box.
        $candidate = @{ Element = $element; Name = $name; Box = $box; Area = $area; Number = $number; Kind = $kind; Offscreen = $element.Current.IsOffscreen }
        # A static-text node can also expose inline fragments. Keep its complete
        # name and enclosing rectangle, rather than treating a word as a lane.
        if (-not $candidates.ContainsKey($key) -or $name.Length -gt $candidates[$key].Name.Length -or
            ($name -ceq $candidates[$key].Name -and $area -gt $candidates[$key].Area)) {
            $candidates[$key] = $candidate
        }
    }
    return $candidates
}

function Get-CaptionHistoryView($Context) {
    $view = $Context.Viewport.Current.BoundingRectangle
    $frame = [MimiPointerSmoke]::ReadBounds($Context.Window)
    if ($view.IsEmpty -or $view.Width -lt 40 -or $view.Height -lt 40 -or
        $view.Left -lt $frame.Left - 1 -or $view.Right -gt $frame.Right + 1 -or
        $view.Top -lt $frame.Top - 1 -or $view.Bottom -gt $frame.Bottom + 1) {
        throw 'The native history scroll viewport is empty or outside the subtitle window.'
    }
    $rows = Get-CaptionHistoryRows $Context
    foreach ($row in $rows.Values) {
        $clip = if ($row.Area -gt 0) { [System.Windows.Rect]::Intersect($row.Box, $view) } else { [System.Windows.Rect]::Empty }
        $row.Visible = -not $row.Offscreen -and -not $clip.IsEmpty -and $clip.Width -gt 1 -and $clip.Height -gt 1
        $row.Clip = $clip
        $row.Full = $row.Visible -and $row.Box.Left -ge $view.Left - 1 -and $row.Box.Right -le $view.Right + 1 -and
            $row.Box.Top -ge $view.Top - 1 -and $row.Box.Bottom -le $view.Bottom + 1
    }
    $scroll = $Context.Scroll.Current
    if ($scroll.VerticalViewSize -le 0) { throw 'The native scroll viewport has no observable content extent.' }
    return @{ View = $view; Rows = $rows; Percent = $scroll.VerticalScrollPercent; ViewSize = $scroll.VerticalViewSize
        ContentExtent = $view.Height * 100.0 / $scroll.VerticalViewSize }
}

function Wait-CaptionHistory([string]$Action, [scriptblock]$Observe) {
    $deadline = [Diagnostics.Stopwatch]::StartNew()
    do {
        try {
            $result = & $Observe
            if ($null -ne $result) { return $result }
        } catch [System.Windows.Automation.ElementNotAvailableException] {
            # UIA can replace a lane when reading changes its compact layout.
        }
        Start-Sleep -Milliseconds 30
    } until ($deadline.ElapsedMilliseconds -ge 5000)
    throw "Native caption history did not complete $Action within 5000ms."
}

function Assert-CaptionHistoryEnvironment($Context) {
    Assert-SettingsHidden
    if ((Read-SmokeMarker $SessionStateFile) -ne 'listening') { throw 'History interaction changed the active session.' }
    if ([MimiPointerSmoke]::ReadBounds($Context.Window).ToString() -cne $Context.RootBounds.ToString() -or
        [MimiPointerSmoke]::FindVisibleControlWindow($Context.Window) -ne $Context.Controls -or
        [MimiPointerSmoke]::ReadBounds($Context.Controls).ToString() -cne $Context.ControlBounds.ToString()) {
        throw 'History scrolling moved, resized, replaced or hid a native subtitle window.'
    }
}

function Get-CaptionHistoryAnchor($State, [double]$Scale) {
    $keys = @($State.Rows.Keys | Where-Object {
        $row = $State.Rows[$_]
        $row.Visible -and $row.Clip.Height -ge 12 * $Scale
    } | Sort-Object { $State.Rows[$_].Box.Top })
    if ($keys.Count -eq 0) { throw 'The native history viewport has no sufficiently visible synthetic lane.' }
    return [string]$keys[0]
}

function Assert-CaptionHistoryAnchor($Before, $After, [string]$Key) {
    if (-not $After.Rows.ContainsKey($Key) -or -not $After.Rows[$Key].Visible -or
        $Before.Rows[$Key].Name -cne $After.Rows[$Key].Name) { throw 'The visible historical caption changed or disappeared.' }
    $a = $Before.Rows[$Key]; $b = $After.Rows[$Key]
    # Compare to the real viewport: quiet chrome changes its screen origin.
    $beforeRange = @(($a.Box.Top - $Before.View.Top), ($a.Clip.Top - $Before.View.Top), $a.Clip.Height, $a.Clip.Width)
    $afterRange = @(($b.Box.Top - $After.View.Top), ($b.Clip.Top - $After.View.Top), $b.Clip.Height, $b.Clip.Width)
    for ($i = 0; $i -lt $beforeRange.Count; $i++) {
        if ([Math]::Abs($beforeRange[$i] - $afterRange[$i]) -gt 1) {
            throw "Native history anchor $Key moved or changed its visible range (before=$beforeRange after=$afterRange)."
        }
    }
}

function Assert-CaptionHistoryReturn($Context, $State, [bool]$Expected) {
    $root = [System.Windows.Automation.AutomationElement]::FromHandle($Context.Window)
    $named = [System.Windows.Automation.PropertyCondition]::new([System.Windows.Automation.AutomationElement]::NameProperty, $Context.ReturnName)
    $buttons = @($root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $named) | Where-Object {
        -not $_.Current.IsOffscreen -and $_.Current.ControlType -eq [System.Windows.Automation.ControlType]::Button
    })
    $count = if ($Expected) { 1 } else { 0 }
    if ($buttons.Count -ne $count) { throw "The native return-to-live action has the wrong visibility (expected=$count observed=$($buttons.Count))." }
    if ($Expected) {
        $box = $buttons[0].Current.BoundingRectangle
        if ($box.IsEmpty -or $box.Width -le 0 -or $box.Height -le 0 -or -not $buttons[0].Current.IsEnabled -or
            $box.Left -lt $State.View.Left - 1 -or $box.Right -gt $State.View.Right + 1 -or
            $box.Top -lt $Context.RootBounds.Top -or $box.Bottom -gt $State.View.Top + 1) {
            throw 'The native return-to-live action is disabled, clipped or covers the reading viewport.'
        }
        $State.ReturnBounds = $box
    }
}

function Save-CaptionHistoryEvidence($Context, $State, [string]$Directory, [string]$Stage) {
    [void][IO.Directory]::CreateDirectory($Directory)
    $frame = [MimiPointerSmoke]::ReadBounds($Context.Window)
    $dpi = [MimiPanelCrop]::GetDpiForWindow($Context.Window)
    if ($dpi -eq 0) { throw 'Could not determine the history capture DPI.' }
    # On setup failure only, retain a caption-only crop without inventing a
    # scroll target. Normal evidence always uses the measured UIA viewport.
    $view = if ($null -ne $State) { $State.View } else {
        $scale = $dpi / 96.0
        [System.Windows.Rect]::new($frame.Left + 12 * $scale, $frame.Top + 60 * $scale,
            $frame.Width - 24 * $scale, $frame.Height - 72 * $scale)
    }
    $left = [int][Math]::Ceiling($view.Left); $top = [int][Math]::Ceiling($view.Top)
    $crop = [Drawing.Rectangle]::new($left, $top, [int][Math]::Floor($view.Right) - $left, [int][Math]::Floor($view.Bottom) - $top)
    if ($crop.Width -lt 8 -or $crop.Height -lt 8) { throw 'The native history capture is empty.' }
    [MimiPanelCrop]::AssertUnobscured($Context.Window, $crop)
    $pixels = [MimiPointerSmoke+PixelPatch]::new($crop.X, $crop.Y, $crop.Width, $crop.Height).Read()
    [MimiPanelCrop]::AssertUnobscured($Context.Window, $crop)
    if ($frame.ToString() -cne [MimiPointerSmoke]::ReadBounds($Context.Window).ToString()) { throw 'The subtitle window moved during history capture.' }
    [MimiPanelCrop]::Save($crop, $pixels, $dpi, (Join-Path $Directory "$Stage.png"))
    $metadata = [ordered]@{ stage = $Stage; dpi = $dpi; root = @($frame.Left, $frame.Top, $frame.Width, $frame.Height)
        viewport = @($view.Left, $view.Top, $view.Width, $view.Height); visible = @() }
    if ($null -ne $State) {
        $metadata.percent = $State.Percent; $metadata.viewSize = $State.ViewSize; $metadata.contentExtent = $State.ContentExtent
        if ($State.ContainsKey('DeliveryEvidence')) { $metadata.deliveryEvidence = $State.DeliveryEvidence }
        if ($State.ContainsKey('ReturnBounds')) {
            $box = $State.ReturnBounds
            $metadata.returnBounds = @($box.Left, $box.Top, $box.Width, $box.Height)
        }
        $metadata.visible = @($State.Rows.Keys | Sort-Object | Where-Object { $State.Rows[$_].Visible } | ForEach-Object {
            $row = $State.Rows[$_]
            @{ lane = $_; offset = $row.Box.Top - $view.Top; bounds = @($row.Box.Left, $row.Box.Top, $row.Box.Width, $row.Box.Height)
                clip = @($row.Clip.Left, $row.Clip.Top, $row.Clip.Width, $row.Clip.Height); full = $row.Full }
        })
    }
    [IO.File]::WriteAllText((Join-Path $Directory "$Stage.json"), ($metadata | ConvertTo-Json -Depth 6), [Text.UTF8Encoding]::new($false))
}

function Get-CaptionHistoryThumb($Context, $State) {
    $scale = $Context.Scale
    $x = [int][Math]::Round($State.View.Right - 3 * $scale)
    $top = [int][Math]::Ceiling($State.View.Top + 2 * $scale)
    $height = [int][Math]::Floor($State.View.Bottom - 2 * $scale) - $top
    $pixels = [MimiPointerSmoke+PixelPatch]::new($x, $top, 1, $height).Read()
    $levels = @($pixels | ForEach-Object { [int]((($_ -band 255) + (($_ -shr 8) -band 255) + (($_ -shr 16) -band 255)) / 3) })
    $sorted = @($levels | Sort-Object)
    $background = $sorted[[int]($height / 5)]
    $predictedLength = $State.View.Height * $State.ViewSize / 100.0
    $predictedCenter = $State.View.Top + ($State.View.Height - $predictedLength) * $State.Percent / 100.0 + $predictedLength / 2
    $runs = @(); $start = -1
    for ($i = 0; $i -le $height; $i++) {
        if ($i -lt $height -and $levels[$i] -ge $background + 12) {
            if ($start -lt 0) { $start = $i }
        } elseif ($start -ge 0) {
            if ($i - $start -ge 6 * $scale -and $predictedCenter -ge $top + $start - 4 * $scale -and
                $predictedCenter -le $top + $i + 4 * $scale) { $runs += @{ Top = $top + $start; Height = $i - $start } }
            $start = -1
        }
    }
    if ($runs.Count -ne 1) {
        throw "No unique rendered scrollbar thumb matched the observed scroll state (segments=$($runs.Count) background=$background peak=$($sorted[-1]))."
    }
    $y = [int][Math]::Floor($runs[0].Top + $runs[0].Height / 2)
    if (-not [MimiPointerSmoke]::IsCaptionPoint($Context.Window, $x, $y)) { throw 'The rendered scrollbar thumb is not owned by the subtitle window.' }
    return @{ X = $x; Y = $y; EndY = [int][Math]::Floor($State.View.Bottom - 2 * $scale); Height = $runs[0].Height; Contrast = $sorted[-1] - $background }
}

function Test-OverlayCaptionHistory([long]$OverlayWindow, [string]$FixtureDirectory, [string]$PanelEvidenceDirectory,
    [ValidateSet('zh', 'de')][string]$Locale = 'zh') {
    $window = [IntPtr]$OverlayWindow
    $controls = [MimiPointerSmoke]::FindVisibleControlWindow($window)
    if ($controls -eq [IntPtr]::Zero) { throw 'The history fixture has no owned language island.' }
    $translationPrefix = [string][char]0x8bd1 + [char]0x6587
    $returnName = if ($Locale -eq 'de') { 'Zur' + [char]0x00fc + 'ck zur Live-Ansicht' } else {
        [string][char]0x56de + [char]0x5230 + [char]0x5b9e + [char]0x65f6
    }
    $context = @{ Window = $window; Controls = $controls; RootBounds = [MimiPointerSmoke]::ReadBounds($window); ReturnName = $returnName
        ControlBounds = [MimiPointerSmoke]::ReadBounds($controls); Scale = [MimiPanelCrop]::GetDpiForWindow($window) / 96.0
        CaptionPattern = '(?<kind>Original|' + $translationPrefix + ') (?<number>[0-9]{2}) '; Viewport = $null; Scroll = $null }
    $state = $null
    try {
        $rows = Wait-CaptionHistory 'the initial 18 synthetic bilingual captions' {
            if ((Read-SmokeMarker (Join-Path $FixtureDirectory 'caption-history-count')) -ne '18') { return $null }
            $sample = Get-CaptionHistoryRows $context
            if ($sample.ContainsKey('source18') -and $sample.ContainsKey('translation18')) { return $sample }
        }
        $ancestor = $rows.source18.Element
        for ($depth = 0; $depth -lt 16 -and $null -ne $ancestor; $depth++) {
            $pattern = $null
            if ($ancestor.TryGetCurrentPattern([System.Windows.Automation.ScrollPattern]::Pattern, [ref]$pattern) -and $pattern.Current.VerticallyScrollable) {
                $context.Viewport = $ancestor; $context.Scroll = $pattern; break
            }
            $ancestor = [System.Windows.Automation.TreeWalker]::RawViewWalker.GetParent($ancestor)
        }
        if ($null -eq $context.Viewport) { throw 'The real caption lane has no observable native scroll viewport.' }
        $state = Wait-CaptionHistory 'the initial visible bilingual caption 18' {
            $sample = Get-CaptionHistoryView $context
            if ($sample.Rows.source18.Full -and $sample.Rows.translation18.Full) { return $sample }
        }
        # Initial layout is complete; all subsequent input must preserve these HWNDs/rects.
        $context.RootBounds = [MimiPointerSmoke]::ReadBounds($window)
        $context.ControlBounds = [MimiPointerSmoke]::ReadBounds($controls)
        Assert-CaptionHistoryEnvironment $context
        $pointX = [int][Math]::Round($state.View.Left + $state.View.Width / 2)
        $pointY = [int][Math]::Round($state.View.Top + $state.View.Height / 2)
        [MimiPointerSmoke]::MoveOverWindow($window, $pointX, $pointY, 'Move into the actual caption history viewport')
        # Calibrate by observed movement, without assuming the OS wheel-line setting.
        for ($detents = 1; $detents -le 10; $detents++) {
            [MimiCaptionDragDesktop]::Wheel(120)
            Start-Sleep -Milliseconds 120
            $state = Get-CaptionHistoryView $context
            if ($state.Percent -lt 99 -and -not $state.Rows.source18.Visible -and -not $state.Rows.translation18.Visible) { break }
        }
        if ($detents -gt 10) { throw 'Physical wheel input did not expose history away from the live caption.' }
        Start-Sleep -Milliseconds 250
        $before = Get-CaptionHistoryView $context
        $anchor = Get-CaptionHistoryAnchor $before $context.Scale
        Assert-CaptionHistoryEnvironment $context
        Assert-CaptionHistoryReturn $context $before $true
        Save-CaptionHistoryEvidence $context $before $PanelEvidenceDirectory 'history-18'
        [IO.File]::WriteAllText((Join-Path $FixtureDirectory 'append-caption-19'), 'append-caption-19', [Text.UTF8Encoding]::new($false))
        $state = Wait-CaptionHistory 'caption 19 delivery while reading' {
            if ((Read-SmokeMarker (Join-Path $FixtureDirectory 'caption-history-count')) -ne '19') { return $null }
            $sample = Get-CaptionHistoryView $context
            if ($sample.Rows.ContainsKey('source19') -and $sample.Rows.ContainsKey('translation19')) {
                $sample.DeliveryEvidence = 'uia-names'; return $sample
            }
            # A renderer may prune offscreen UIA nodes. The actual scrollable
            # content must then grow, with the visible anchor still unchanged.
            if ($sample.ContentExtent -gt $before.ContentExtent + $context.Scale) {
                $sample.DeliveryEvidence = 'content-extent'; return $sample
            }
        }
        $deliveryEvidence = $state.DeliveryEvidence
        Start-Sleep -Milliseconds 250
        $state = Get-CaptionHistoryView $context
        $state.DeliveryEvidence = $deliveryEvidence
        Assert-CaptionHistoryAnchor $before $state $anchor
        if ($state.Rows.source19.Visible -or $state.Rows.translation19.Visible) { throw 'Caption 19 replaced the historical reading viewport.' }
        Assert-CaptionHistoryEnvironment $context
        Assert-CaptionHistoryReturn $context $state $true
        Save-CaptionHistoryEvidence $context $state $PanelEvidenceDirectory 'history-19'
        Write-Host "Native history kept $anchor at offset=$($state.Rows[$anchor].Box.Top - $state.View.Top) after caption 19 (wheelDetents=$detents)."

        [void][MimiPointerSmoke]::MoveOutside($window, $controls)
        Start-Sleep -Milliseconds 250
        $quiet = Get-CaptionHistoryView $context
        Assert-CaptionHistoryAnchor $state $quiet $anchor
        Assert-CaptionHistoryReturn $context $quiet $true
        Assert-CaptionHistoryEnvironment $context
        $pointX = [int][Math]::Round($quiet.View.Left + $quiet.View.Width / 2)
        $pointY = [int][Math]::Round($quiet.View.Top + $quiet.View.Height / 2)
        [MimiPointerSmoke]::MoveOverWindow($window, $pointX, $pointY, 'Return to the unchanged historical reading viewport')
        Start-Sleep -Milliseconds 250
        $state = Get-CaptionHistoryView $context
        Assert-CaptionHistoryAnchor $quiet $state $anchor
        Assert-CaptionHistoryReturn $context $state $true

        $thumb = Get-CaptionHistoryThumb $context $state
        [MimiPointerSmoke]::MoveOverWindow($window, $thumb.X, $thumb.Y, 'Press the rendered history scrollbar thumb')
        try {
            [MimiCaptionDragDesktop]::SetLeftButton($true)
            if (-not [MimiCaptionDragDesktop]::LeftDown()) { throw 'The history scrollbar did not receive a held physical left button.' }
            for ($step = 1; $step -le 8; $step++) {
                [MimiPointerSmoke]::MoveAndVerify($thumb.X, [int][Math]::Round($thumb.Y + ($thumb.EndY - $thumb.Y) * $step / 8))
                Start-Sleep -Milliseconds 30
                Assert-CaptionHistoryEnvironment $context
            }
        } finally { [MimiCaptionDragDesktop]::SetLeftButton($false) }
        if ([MimiCaptionDragDesktop]::LeftDown()) { throw 'Windows retained the history scrollbar button after release.' }
        $state = Wait-CaptionHistory 'the physical scrollbar reaching the real bottom' {
            $sample = Get-CaptionHistoryView $context
            if ($sample.Percent -ge 99.99 -and $sample.Rows.source19.Full -and $sample.Rows.translation19.Full) { return $sample }
        }
        Assert-CaptionHistoryReturn $context $state $false
        [IO.File]::WriteAllText((Join-Path $FixtureDirectory 'append-caption-20'), 'append-caption-20', [Text.UTF8Encoding]::new($false))
        $state = Wait-CaptionHistory 'caption 20 following after scrollbar release' {
            if ((Read-SmokeMarker (Join-Path $FixtureDirectory 'caption-history-count')) -ne '20') { return $null }
            $sample = Get-CaptionHistoryView $context
            if ($sample.Rows.ContainsKey('source20') -and $sample.Rows.ContainsKey('translation20') -and
                $sample.Percent -ge 99.99 -and $sample.Rows.source20.Full -and $sample.Rows.translation20.Full) { return $sample }
        }
        Start-Sleep -Milliseconds 400
        $state = Get-CaptionHistoryView $context
        if ($state.Percent -lt 99.99 -or -not $state.Rows.source20.Full -or -not $state.Rows.translation20.Full) { throw 'The resumed live caption did not remain fully in view.' }
        Assert-CaptionHistoryEnvironment $context
        Assert-CaptionHistoryReturn $context $state $false
        Save-CaptionHistoryEvidence $context $state $PanelEvidenceDirectory 'live-20'
        Write-Host "Native history passed: physical wheel preserved $anchor on append; rendered thumb=$($thumb.Height)px contrast=$($thumb.Contrast); scrollbar release restored bilingual caption 20 at the bottom, same windows, Settings hidden, listening."
    } catch {
        $failure = $_
        try {
            if ($null -ne $context.Viewport) { $state = Get-CaptionHistoryView $context }
            Save-CaptionHistoryEvidence $context $state $PanelEvidenceDirectory 'history-failed'
        } catch { Write-Warning 'Could not save the bounded synthetic history failure capture.' }
        throw $failure
    }
}
