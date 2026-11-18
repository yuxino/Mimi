# Dot-sourced only by the isolated caption-drag Windows UI-test process.
# The lane exposes its complete accessible name; its painted child is aria-hidden.
function Get-CaptionDragPoint {
    $byCaption = [System.Windows.Automation.PropertyCondition]::new(
        [System.Windows.Automation.AutomationElement]::NameProperty,
        'Mimi system audio test: can you see the shared screen?')
    $deadline = [Diagnostics.Stopwatch]::StartNew()
    $lastSample = 'lane=unavailable'
    do {
        try {
            $root = [System.Windows.Automation.AutomationElement]::FromHandle($overlay)
            foreach ($lane in $root.FindAll([System.Windows.Automation.TreeScope]::Descendants, $byCaption)) {
                $bounds = $lane.Current.BoundingRectangle
                if ($lane.Current.IsOffscreen -or $bounds.IsEmpty) { continue }
                $frame = [MimiPointerSmoke]::ReadBounds($overlay)
                # Only sample the lane interior, clipped to this native window.
                # A central patch excludes the viewport edges and scrollbar.
                $left = [int][Math]::Ceiling([Math]::Max($bounds.Left + 2, $frame.Left + 4))
                $top = [int][Math]::Ceiling([Math]::Max($bounds.Top + 2, $frame.Top + 4))
                $right = [int][Math]::Floor([Math]::Min($bounds.Right - 2, $frame.Right - 8))
                $bottom = [int][Math]::Floor([Math]::Min($bounds.Bottom - 2, $frame.Bottom - 4))
                $width = [Math]::Min(192, $right - $left)
                $height = [Math]::Min(48, $bottom - $top)
                if ($width -lt 8 -or $height -lt 8) { continue }
                $left += [int](($right - $left - $width) / 2)
                $top += [int](($bottom - $top - $height) / 2)
                $pixels = [MimiPointerSmoke+PixelPatch]::new($left, $top, $width, $height).Read()
                $brightness = [int[]]::new($pixels.Length)
                for ($i = 0; $i -lt $pixels.Length; $i++) {
                    $brightness[$i] = [int]((($pixels[$i] -band 255) + (($pixels[$i] -shr 8) -band 255) + (($pixels[$i] -shr 16) -band 255)) / 3)
                }
                $sorted = $brightness.Clone()
                [Array]::Sort($sorted)
                $background = $sorted[[int]($sorted.Length / 2)]
                $lastSample = 'lane={0}x{1} patch={2}x{3} background={4} peak={5}' -f [int]$bounds.Width, [int]$bounds.Height, $width, $height, $background, $sorted[-1]
                if ($background -gt 120) { continue }
                $best = $null
                $distance = [double]::PositiveInfinity
                for ($py = 2; $py -lt ($height - 2); $py++) {
                    for ($px = 2; $px -lt ($width - 2); $px++) {
                        $i = $py * $width + $px
                        if ($brightness[$i] -lt 180 -or $brightness[$i] - $background -lt 70) { continue }
                        $r = ($pixels[$i] -shr 16) -band 255
                        $g = ($pixels[$i] -shr 8) -band 255
                        $b = $pixels[$i] -band 255
                        if ([Math]::Max($r, [Math]::Max($g, $b)) - [Math]::Min($r, [Math]::Min($g, $b)) -gt 24) { continue }
                        # Reject isolated bright pixels; a visible glyph has
                        # another bright pixel immediately beside its stroke.
                        if ($brightness[$i - 1] -lt 160 -and $brightness[$i + 1] -lt 160 -and
                            $brightness[$i - $width] -lt 160 -and $brightness[$i + $width] -lt 160) { continue }
                        $score = [Math]::Pow($px - ($width - 1) / 2, 2) + [Math]::Pow($py - ($height - 1) / 2, 2)
                        if ($score -ge $distance -or -not [MimiPointerSmoke]::IsCaptionPoint($overlay, $left + $px, $top + $py)) { continue }
                        $distance = $score
                        $best = @{ X = $left + $px; Y = $top + $py; Width = [int]$bounds.Width; Height = [int]$bounds.Height;
                            Brightness = $brightness[$i]; Contrast = $brightness[$i] - $background }
                    }
                }
                if ($null -ne $best) { return $best }
            }
        }
        catch [System.Windows.Automation.ElementNotAvailableException] {
            # The first accessible caption can be replaced during setup.
        }
        Start-Sleep -Milliseconds 25
    } until ($deadline.ElapsedMilliseconds -ge 2500)
    throw "The named synthetic caption had no verified bright glyph on its dark background ($lastSample)."
}

function Wait-CaptionDragPosition($Root, $Child, [IntPtr]$Controls, [int]$X, [int]$Y, [int]$StableMs = 0) {
    $deadline = [Diagnostics.Stopwatch]::StartNew()
    $stableSince = -1
    do {
        Assert-SettingsHidden
        if ((Read-SmokeMarker $SessionStateFile) -ne 'listening') { throw 'Caption dragging changed the active session.' }
        if ([MimiPointerSmoke]::FindVisibleControlWindow($overlay) -ne $Controls) { throw 'Caption dragging replaced or hid the owned control window.' }
        $main = [MimiPointerSmoke]::ReadBounds($overlay)
        $childNow = [MimiPointerSmoke]::ReadBounds($Controls)
        if ($main.Width -ne $Root.Width -or $main.Height -ne $Root.Height -or
            $childNow.Width -ne $Child.Width -or $childNow.Height -ne $Child.Height) { throw 'Caption dragging resized a native window.' }
        $matched = $main.Left -eq $Root.Left + $X -and $main.Top -eq $Root.Top + $Y -and
            $childNow.Left -eq $Child.Left + $X -and $childNow.Top -eq $Child.Top + $Y
        if ($matched) {
            if ($stableSince -lt 0) { $stableSince = $deadline.ElapsedMilliseconds }
            if ($deadline.ElapsedMilliseconds - $stableSince -ge $StableMs) { return }
        } else { $stableSince = -1 }
        Start-Sleep -Milliseconds 25
    } until ($deadline.ElapsedMilliseconds -ge 2500)
    throw "Caption drag did not reach/stay at physical offset ($X,$Y): root=$main control=$childNow."
}

function Test-NativeCaptionDrag([IntPtr]$Controls) {
    $point = Get-CaptionDragPoint
    Write-Output ('Native caption glyph lane={0}x{1} brightness={2} contrast={3} point=({4},{5}).' -f $point.Width, $point.Height, $point.Brightness, $point.Contrast, $point.X, $point.Y)
    $main = [MimiPointerSmoke]::ReadBounds($overlay)
    $child = [MimiPointerSmoke]::ReadBounds($Controls)
    [MimiPointerSmoke]::ClickOverlay($overlay, $point.X, $point.Y, 'Click the visible caption without dragging')
    Wait-CaptionDragPosition $main $child $Controls 0 0 -StableMs 400
    Write-Output "Native caption click kept root=$main control=$child unchanged for 400ms."
    $delta = [MimiPointerSmoke]::CaptionDragDelta($overlay, $Controls)
    foreach ($direction in @(1, -1)) {
        $point = Get-CaptionDragPoint
        [MimiPointerSmoke]::MoveOverWindow($overlay, $point.X, $point.Y, 'Press a verified caption glyph for native dragging')
        $dragFailure = $null
        $releaseFailure = $null
        $heldFrames = [Collections.Generic.List[object]]::new()
        try {
            [MimiPointerSmoke]::SetCaptionButton($true)
            # A deliberate held gesture, including a short pause mid-drag;
            # no internal move-loop flag is required before physical motion.
            Start-Sleep -Milliseconds 80
            for ($step = 1; $step -le 8; $step++) {
                [MimiPointerSmoke]::MoveAndVerify([int]($point.X + $direction * $delta.X * $step / 8),
                    [int]($point.Y + $direction * $delta.Y * $step / 8))
                Start-Sleep -Milliseconds $(if ($step -eq 4) { 200 } else { 20 })
                try {
                    $heldFrames.Add(@{ Step = $step; Root = [MimiPointerSmoke]::ReadBounds($overlay);
                        Control = [MimiPointerSmoke]::ReadBounds($Controls) })
                }
                catch { $heldFrames.Add(@{ Step = $step; Root = 'unavailable'; Control = 'unavailable' }) }
            }
            $x = if ($direction -eq 1) { $delta.X } else { 0 }
            $y = if ($direction -eq 1) { $delta.Y } else { 0 }
            Wait-CaptionDragPosition $main $child $Controls $x $y
        }
        catch {
            $dragFailure = $_
            foreach ($frame in $heldFrames) {
                Write-Output ('Native caption drag trace step={0} root={1} control={2}' -f $frame.Step, $frame.Root, $frame.Control)
            }
            try { Write-Output ('Native caption drag diagnostic phase=held ' + [MimiPointerSmoke]::CaptionDragObservation($overlay, $Controls)) }
            catch { Write-Output 'Native caption drag diagnostic unavailable phase=held.' }
        }
        finally {
            try { [MimiPointerSmoke]::SetCaptionButton($false) }
            catch { $releaseFailure = $_ }
        }
        if ($null -ne $dragFailure -or $null -ne $releaseFailure) {
            # Observe whether an outline-only system drag commits on release.
            # These samples never turn the original held-geometry failure into a pass.
            $released = [Diagnostics.Stopwatch]::StartNew()
            for ($sample = 0; $sample -lt 3; $sample++) {
                if ($sample -gt 0) { Start-Sleep -Milliseconds 200 }
                try {
                    Write-Output ('Native caption drag diagnostic phase=released elapsed_ms={0} release_input_accepted={1} {2}' -f
                        $released.ElapsedMilliseconds, ($null -eq $releaseFailure), [MimiPointerSmoke]::CaptionDragObservation($overlay, $Controls))
                }
                catch { Write-Output 'Native caption drag diagnostic unavailable phase=released.' }
            }
            if ($null -ne $dragFailure) { throw $dragFailure }
            throw $releaseFailure
        }
        # Includes the native geometry manager's 350ms persistence debounce.
        Wait-CaptionDragPosition $main $child $Controls $x $y -StableMs 400
        $observedRoot = [MimiPointerSmoke]::ReadBounds($overlay)
        $observedChild = [MimiPointerSmoke]::ReadBounds($Controls)
        Write-Output "Native caption drag offset=($x,$y), root=$observedRoot control=$observedChild, release stable=400ms."
    }
    Write-Output 'Native caption drag passed: same root and control HWNDs moved together and returned exactly, click kept position, Settings stayed hidden, session listening.'
}
