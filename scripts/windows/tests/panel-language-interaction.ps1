# Native, credential-free panel acceptance. The caller owns the process, DPI
# context, private preferences and screenshot/layout evidence helper.
Add-Type @'
using System;
using System.Runtime.InteropServices;

public static class MimiPanelKeyboard {
    [StructLayout(LayoutKind.Sequential)]
    private struct MouseInput { internal int X, Y; internal uint Data, Flags, Time; internal UIntPtr Extra; }
    [StructLayout(LayoutKind.Sequential)]
    private struct KeyboardInput { internal ushort Key, Scan; internal uint Flags, Time; internal UIntPtr Extra; }
    [StructLayout(LayoutKind.Explicit)]
    private struct InputData {
        [FieldOffset(0)] internal MouseInput Mouse;
        [FieldOffset(0)] internal KeyboardInput Keyboard;
    }
    [StructLayout(LayoutKind.Sequential)]
    private struct Input { internal uint Type; internal InputData Data; }
    [StructLayout(LayoutKind.Sequential)]
    private struct Rect { internal int Left, Top, Right, Bottom; }
    [StructLayout(LayoutKind.Sequential)]
    private struct GuiInfo {
        internal uint Size, Flags;
        internal IntPtr Active, Focus, Capture, MenuOwner, MoveSize, Caret;
        internal Rect CaretBounds;
    }
    [DllImport("user32.dll")] private static extern IntPtr GetForegroundWindow();
    [DllImport("user32.dll")] private static extern IntPtr GetAncestor(IntPtr window, uint flags);
    [DllImport("user32.dll")] private static extern bool GetGUIThreadInfo(uint thread, ref GuiInfo info);
    [DllImport("user32.dll")] private static extern uint SendInput(uint count, Input[] inputs, int size);

    [DllImport("user32.dll")] private static extern bool GetWindowRect(IntPtr window, out Rect rect);
    [DllImport("user32.dll", SetLastError = true)] private static extern bool SetWindowPos(IntPtr window, IntPtr after, int x, int y, int width, int height, uint flags);
    public sealed class KeyObservation {
        public long BeginWallMs, EndWallMs;
        public int[] BeforeBounds, AfterBounds;
        public uint Accepted;
    }
    private static bool observeNext;
    public static KeyObservation LastObservedKey;
    public static void ObserveNextKey() { observeNext = true; LastObservedKey = null; }
    public static int[] Bounds(IntPtr controls) {
        Rect rect;
        return GetWindowRect(controls, out rect) ? new[] { rect.Left, rect.Top, rect.Right - rect.Left, rect.Bottom - rect.Top } : null;
    }
    public static void ResizeOnce(IntPtr controls) {
        var bounds = Bounds(controls);
        if (bounds == null || bounds[2] <= 0 || bounds[3] <= 0 ||
            !SetWindowPos(controls, IntPtr.Zero, 0, 0, bounds[2], bounds[3] + 12, 0x0002u | 0x0004u | 0x0010u))
            throw new InvalidOperationException("Could not apply the separate 12px native resize.");
    }

    public static bool OwnsFocus(IntPtr controls) {
        var gui = new GuiInfo { Size = (uint)Marshal.SizeOf(typeof(GuiInfo)) };
        return GetAncestor(GetForegroundWindow(), 2) == controls && GetGUIThreadInfo(0, ref gui)
            && gui.Focus != IntPtr.Zero && GetAncestor(gui.Focus, 2) == controls;
    }

    public static void Key(IntPtr controls, ushort key) {
        if (!OwnsFocus(controls))
            throw new InvalidOperationException("The language control window did not own native keyboard focus.");
        uint extended = key == 0x23 || key == 0x24 || key == 0x28 ? 1u : 0u;
        var up = new Input { Type = 1, Data = new InputData { Keyboard = new KeyboardInput { Key = key, Flags = extended | 2u } } };
        var inputs = new[] {
            new Input { Type = 1, Data = new InputData { Keyboard = new KeyboardInput { Key = key, Flags = extended } } }, up
        };
        int size = Marshal.SizeOf(typeof(Input));
        var observation = observeNext ? new KeyObservation { BeforeBounds = Bounds(controls), BeginWallMs = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds() } : null;
        observeNext = false;
        uint sent = SendInput(2, inputs, size);
        if (observation != null) {
            observation.EndWallMs = DateTimeOffset.UtcNow.ToUnixTimeMilliseconds();
            observation.Accepted = sent;
            observation.AfterBounds = Bounds(controls);
            LastObservedKey = observation;
        }
        if (sent == 1) SendInput(1, new[] { up }, size);
        if (sent != 2) throw new InvalidOperationException("Windows did not accept the panel keyboard input pair.");
    }
}
'@

function Test-PanelLanguage($Controls, $Labels, $EvidencePath) {
    $controlsHandle = [IntPtr]$Controls
    $scope = [System.Windows.Automation.TreeScope]::Descendants
    $typeProperty = [System.Windows.Automation.AutomationElement]::ControlTypeProperty
    $nameProperty = [System.Windows.Automation.AutomationElement]::NameProperty
    $preferencesPath = Join-Path $FixtureDirectory 'preferences.json'
    $observation = @{ active = $false; key = $null; firstMenuRead = $null; firstReopen = $null; baselinePassed = $false; snapshots = [Collections.Generic.List[object]]::new() }

    # UIA may project aria-activedescendant as the focused ListItem instead of
    # exposing the DOM search input. Require the actual target popup hierarchy.
    function Get-PanelPopupFocus($Menu, $Focused) {
        if (-not $Menu -or -not $Focused -or -not [MimiPanelKeyboard]::OwnsFocus($controlsHandle)) { return 'other-panel-element' }
        if ($Menu.List.Current.IsOffscreen) { return 'other-panel-element' }
        $root = [System.Windows.Automation.AutomationElement]::FromHandle($controlsHandle)
        $listType = [System.Windows.Automation.PropertyCondition]::new($typeProperty, [System.Windows.Automation.ControlType]::List)
        $lists = @($root.FindAll($scope, $listType) | Where-Object { -not $_.Current.IsOffscreen })
        if ($lists.Count -ne 1 -or -not $lists[0].Equals($Menu.List)) { return 'other-panel-element' }
        $searchByName = [System.Windows.Automation.PropertyCondition]::new($nameProperty, $Labels.searchLanguages)
        $searches = @($root.FindAll($scope, $searchByName) | Where-Object {
            -not $_.Current.IsOffscreen -and $_.Current.ControlType -in @([System.Windows.Automation.ControlType]::ComboBox, [System.Windows.Automation.ControlType]::Edit)
        })
        if ($searches.Count -eq 1 -and $Focused.Equals($searches[0])) { return 'search' }
        if ($Focused.Current.ControlType -ne [System.Windows.Automation.ControlType]::ListItem) { return 'other-panel-element' }
        $ancestor = $Focused
        for ($depth = 0; $depth -lt 16 -and $null -ne $ancestor; $depth++) {
            if ($ancestor.Equals($Menu.List)) { return 'active-option' }
            $ancestor = [System.Windows.Automation.TreeWalker]::RawViewWalker.GetParent($ancestor)
        }
        return 'other-panel-element'
    }

    function Read-PanelFocusObservation([string]$Phase) {
        $codes = Read-PanelLanguageCodes
        $menu = Read-PanelLanguageMenu $Labels.target
        $focused = [System.Windows.Automation.AutomationElement]::FocusedElement
        $target = Get-PanelLanguageField $Labels.target
        $source = Get-PanelLanguageField $Labels.source
        $focus = 'none'
        $focusType = $null
        $focusBox = $null
        if ($null -ne $focused) {
            $focusType = $focused.Current.ControlType.ProgrammaticName
            $box = $focused.Current.BoundingRectangle
            $focusBox = @($box.X, $box.Y, $box.Width, $box.Height)
            $focus = if ($target -and $focused.Equals($target)) { 'target' }
                elseif ($source -and $focused.Equals($source)) { 'source' }
                else { Get-PanelPopupFocus $menu $focused }
        }
        $row = @{ phase = $Phase; wallMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
            sourceCode = $codes.source; targetCode = $codes.target; focus = $focus; focusType = $focusType; focusBounds = $focusBox
            nativeOwnsFocus = [MimiPanelKeyboard]::OwnsFocus($controlsHandle)
            childBounds = [MimiPanelKeyboard]::Bounds($controlsHandle); rootBounds = [MimiPanelKeyboard]::Bounds($overlay)
            menu = $(if ($menu) { @{ present = $true; index = $menu.Index; options = $menu.Names.Count } } else { @{ present = $false } }) }
        $observation.snapshots.Add($row)
        return $row
    }

    function Save-PanelFocusObservation {
        [void][IO.Directory]::CreateDirectory($EvidencePath)
        if ($observation.mode -ne 'controlled-resize-only' -and $null -eq $observation.key) { $observation.key = [MimiPanelKeyboard]::LastObservedKey }
        [IO.File]::WriteAllText((Join-Path $EvidencePath 'ru-target-end-focus.json'),
            ($observation | ConvertTo-Json -Depth 8), [Text.UTF8Encoding]::new($false))
    }

    function Test-SeparatePanelResize {
        $control = @{ separate = $true; started = $false; completed = $false; arrowOpenedMenu = $false }
        $observation.controlledResize = $control
        try {
            $field = Get-PanelLanguageField $Labels.target
            if (-not $field) { throw 'The separate resize has no available target trigger.' }
            $closed = Get-PanelPickerValueEvidence $field 'target'
            $codes = Read-PanelLanguageCodes
            if ($codes.source -cne 'en' -or $codes.target -cne 'fa') { throw 'The separate resize requires persisted en and fa.' }
            $menu = Read-PanelLanguageMenu $Labels.target
            if (-not $menu) {
                $box = $field.Current.BoundingRectangle
                [MimiPointerSmoke]::ClickOverlay($controlsHandle, [int]($box.X + $box.Width / 2), [int]($box.Y + $box.Height / 2), 'Open the controlled target search')
                $control.openMethod = 'physical-trigger-click'
            } else { $control.openMethod = 'existing-menu' }
            $menu = Wait-PanelLanguageState 'separate-resize-open-menu' { Read-PanelLanguageMenu $Labels.target }
            $control.selectedMatchesClosed = [string]::Equals($closed.value, $menu.Selected, [StringComparison]::Ordinal)
            $before = Read-PanelFocusObservation 'separate-resize-before'
            if ($before.focus -notin @('search', 'active-option') -or -not $before.nativeOwnsFocus -or
                -not $before.menu.present -or -not $control.selectedMatchesClosed -or $before.targetCode -cne 'fa') {
                throw 'The separate resize requires owned target search or its actual current option and persisted fa.'
            }
            $control.started = $true
            $control.beginWallMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
            [MimiPanelKeyboard]::ResizeOnce($controlsHandle)
            $control.endWallMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
            $null = Wait-PanelLanguageState 'separate-resize-closed-menu' { -not (Read-PanelLanguageMenu $Labels.target) }
            $after = Read-PanelFocusObservation 'separate-resize-after'
            $control.focusAfterResize = $after.focus
            $control.focusTypeAfterResize = $after.focusType
            $control.targetCodeAfterResize = $after.targetCode
            # One real key, with no click/refocus workaround after the resize.
            [MimiPanelKeyboard]::ObserveNextKey()
            [MimiPanelKeyboard]::Key($controlsHandle, 0x28)
            $control.arrowKey = [MimiPanelKeyboard]::LastObservedKey
            try {
                $null = Wait-PanelLanguageState 'target-controlled-reopen' {
                    $current = Read-PanelLanguageMenu $Labels.target
                    if ($current -and [string]::Equals($current.Selected, $closed.value, [StringComparison]::Ordinal)) { $current }
                }
                $control.arrowOpenedMenu = $true
            } catch { $control.arrowFailureType = $_.Exception.GetType().Name }
            $null = Read-PanelFocusObservation 'separate-resize-after-one-ArrowDown'
            $control.completed = $true
        } catch {
            $control.failureType = $_.Exception.GetType().Name
            Write-Output 'The separate resize observation did not complete; original baseline outcome is retained.'
        }
    }

    function Wait-PanelLanguageState([string]$Phase, [scriptblock]$Observe) {
        $deadline = [Diagnostics.Stopwatch]::StartNew()
        do {
            Assert-SettingsHidden
            try {
                if ((Read-SmokeMarker $SessionStateFile) -eq 'listening') {
                    $result = & $Observe
                    if ($result) { return $result }
                }
            }
            catch [System.Windows.Automation.ElementNotAvailableException] {
                # A language snapshot can replace nodes while the tree updates.
            }
            Start-Sleep -Milliseconds 25
        } until ($deadline.ElapsedMilliseconds -ge 5000)
        $detail = Get-PanelLanguageFailureDetail $Phase
        throw "Native panel language check timed out (locale=$($Labels.locale) phase=$Phase $detail)."
    }

    function Get-PanelLanguageField([string]$Name) {
        $byName = [System.Windows.Automation.PropertyCondition]::new($nameProperty, $Name)
        $root = [System.Windows.Automation.AutomationElement]::FromHandle($controlsHandle)
        foreach ($element in $root.FindAll($scope, $byName)) {
            if ($element.Current.ControlType -in @([System.Windows.Automation.ControlType]::ComboBox, [System.Windows.Automation.ControlType]::Button) -and
                $element.Current.IsEnabled -and -not $element.Current.IsOffscreen) { return $element }
        }
    }

    function Read-PanelLanguageCodes {
        try { $json = Read-SmokeMarker $preferencesPath }
        catch { throw "Could not read isolated panel preferences (locale=$($Labels.locale))." }
        if (-not $json) { return }
        try { $preferences = ConvertFrom-Json -InputObject $json -ErrorAction Stop }
        catch { throw "The isolated panel preferences are not valid JSON (locale=$($Labels.locale))." }
        $codes = @{ source = $preferences.source_language; target = $preferences.target_language }
        foreach ($kind in @('source', 'target')) {
            $code = $codes.$kind
            if ($null -eq $code) { throw "The isolated panel preferences have a missing $kind language code (locale=$($Labels.locale))." }
            if ($code -isnot [string] -or $code -cnotmatch '\A(?:auto|original|[a-z]{2,3}(?:[-_][A-Za-z0-9]{2,8})?)\z') {
                throw "The isolated panel preferences have an invalid $kind language code (locale=$($Labels.locale))."
            }
        }
        return $codes
    }

    function Read-PanelLanguageMenu([string]$Name) {
        $root = [System.Windows.Automation.AutomationElement]::FromHandle($controlsHandle)
        $condition = [System.Windows.Automation.AndCondition]::new([System.Windows.Automation.Condition[]]@(
            [System.Windows.Automation.PropertyCondition]::new($nameProperty, $Name),
            [System.Windows.Automation.PropertyCondition]::new($typeProperty, [System.Windows.Automation.ControlType]::List)))
        $list = $root.FindFirst($scope, $condition)
        $offscreen = $null -ne $list -and $list.Current.IsOffscreen
        $recordThisRead = $observation.active -and $null -eq $observation.firstMenuRead
        if ($recordThisRead) {
            # No extra UIA query before the first physical key or its normal menu read.
            $observation.firstMenuRead = @{ wallMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
                present = $null -ne $list; offscreen = $offscreen
                childBounds = [MimiPanelKeyboard]::Bounds($controlsHandle)
                nativeOwnsFocus = [MimiPanelKeyboard]::OwnsFocus($controlsHandle) }
        }
        if ($null -eq $list -or $offscreen) { return }
        $items = @()
        $selected = @()
        $itemCondition = [System.Windows.Automation.PropertyCondition]::new($typeProperty, [System.Windows.Automation.ControlType]::ListItem)
        foreach ($item in $list.FindAll($scope, $itemCondition)) {
            if (-not $item.Current.IsEnabled) { continue }
            $selection = $null
            if (-not $item.TryGetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern, [ref]$selection)) {
                throw 'A native language option did not expose its selection state.'
            }
            $index = $items.Count
            $items += $item.Current.Name
            if ($selection.Current.IsSelected) { $selected += $index }
        }
        if ($recordThisRead) {
            $observation.firstMenuRead.readEndWallMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
            $observation.firstMenuRead.options = $items.Count
            $observation.firstMenuRead.selectedIndices = @($selected)
        }
        if ($items.Count -gt 1 -and $items.Count -le 256 -and $selected.Count -eq 1) {
            return @{ Names = $items; Index = $selected[0]; Selected = $items[$selected[0]]; List = $list }
        }
    }

    function Test-PanelLanguageClosed([string]$Name) {
        $field = Get-PanelLanguageField $Name
        if ($null -eq $field -or -not [MimiPanelKeyboard]::OwnsFocus($controlsHandle)) { return $false }
        $focused = [System.Windows.Automation.AutomationElement]::FocusedElement
        if ($null -eq $focused -or -not $focused.Equals($field)) { return $false }
        $root = [System.Windows.Automation.AutomationElement]::FromHandle($controlsHandle)
        $anyList = [System.Windows.Automation.PropertyCondition]::new($typeProperty, [System.Windows.Automation.ControlType]::List)
        foreach ($list in $root.FindAll($scope, $anyList)) {
            if (-not $list.Current.IsOffscreen) { return $false }
        }
        return $true
    }

    # Only sample after the deadline. Successive native keys must keep their
    # original timing, including the first ArrowDown after a preference save.
    function Get-PanelLanguageFailureDetail([string]$Phase) {
        $details = @()
        try {
            $codes = Read-PanelLanguageCodes
            if ($codes) { $details += "source_code=$($codes.source) target_code=$($codes.target)" }
            else { $details += 'codes=missing' }
        } catch { $details += 'codes=unreadable' }

        $parts = $Phase.Split('-')
        $fieldKind = $parts[0]
        if ($fieldKind -notin @('source', 'target')) { return $details -join ' ' }
        $wanted = switch ($parts[-1]) {
            'Home' { 0 }
            'End' { $names.Count - 1 }
            'Restore' { $initialIndex }
        }
        if ($null -ne $wanted) { $details += "expected_index=$wanted expected_name='$($names[$wanted])'" }

        try {
            $root = [System.Windows.Automation.AutomationElement]::FromHandle($controlsHandle)
            $ownsFocus = [MimiPanelKeyboard]::OwnsFocus($controlsHandle)
            $focused = if ($ownsFocus) { [System.Windows.Automation.AutomationElement]::FocusedElement }
            $focus = if ($ownsFocus) { 'other-panel-element' } else { 'outside-panel' }
            $menu = 'absent'
            $field = 'absent'
            $byName = [System.Windows.Automation.PropertyCondition]::new($nameProperty, $Labels.$fieldKind)
            foreach ($element in $root.FindAll($scope, $byName)) {
                $type = $element.Current.ControlType
                if ($type -in @([System.Windows.Automation.ControlType]::ComboBox, [System.Windows.Automation.ControlType]::Button)) {
                    $field = "enabled=$($element.Current.IsEnabled) offscreen=$($element.Current.IsOffscreen)"
                    if ($focused -and $focused.Equals($element)) { $focus = 'field' }
                    try {
                        $value = Get-PanelPickerValueEvidence $element $fieldKind
                        $field += " value='$($value.value)'"
                    } catch { $field += ' value=unreadable' }
                } elseif ($type -eq [System.Windows.Automation.ControlType]::List) {
                    if ($element.Current.IsOffscreen) {
                        if ($menu -eq 'absent') { $menu = 'offscreen' }
                        continue
                    }
                    $index = 0
                    $selected = @()
                    $items = [System.Windows.Automation.PropertyCondition]::new($typeProperty, [System.Windows.Automation.ControlType]::ListItem)
                    foreach ($item in $element.FindAll($scope, $items)) {
                        if (-not $item.Current.IsEnabled) { continue }
                        $selection = $null
                        if ($item.TryGetCurrentPattern([System.Windows.Automation.SelectionItemPattern]::Pattern, [ref]$selection)) {
                            if ($selection.Current.IsSelected) { $selected += $index }
                        } else { $selected += "unknown@$index" }
                        if ($focused -and $focused.Equals($item)) { $focus = "option[$index]" }
                        $index++
                    }
                    $menu = "open options=$index selected_indices=[$($selected -join ',')]"
                }
            }
            $details += "field=($field) menu=($menu) uia_focus=$focus"
        } catch { $details += "uia_unavailable=$($_.Exception.GetType().Name)" }
        return $details -join ' '
    }

    $controlledMarker = Join-Path $FixtureDirectory 'panel-focus-controlled-resize-only'
    if ($env:RUNNER_ENVIRONMENT -ceq 'github-hosted' -and $env:MIMI_UI_TEST -ceq '1' -and
        (Test-Path -LiteralPath $controlledMarker -PathType Leaf) -and (Get-Item -LiteralPath $controlledMarker).Length -eq 0) {
        $observation.mode = 'controlled-resize-only'
        $observation.baselinePassed = $null
        $observation.historicalBaseline = @{ run = 44903824780; passed = $true; notRepeated = $true }
        try {
            $codes = Read-PanelLanguageCodes
            if ($codes.source -cne 'en' -or $codes.target -cne 'fa') { throw 'The controlled-only fixture must begin with persisted en and fa.' }
            $island = Use-OverlayControl $Labels.openControls 'listening' 'Locate the controlled language island' -Surface $controlsHandle -NameSuffix -BoundsOnly -WaitMs 5000
            Move-OverControlBounds $controlsHandle $island 'Reveal the controlled language island'
            Start-Sleep -Milliseconds 240
            Use-OverlayControl $Labels.openControls 'listening' 'Open the controlled language panel' -Surface $controlsHandle -NameSuffix -WaitMs 5000
            Use-OverlayControl $Labels.moreSettings 'listening' 'Wait for the controlled language panel' -Surface $controlsHandle -WaitOnly -WaitMs 5000
            $null = Wait-PanelLanguageState 'controlled-panel-focus' { [MimiPanelKeyboard]::OwnsFocus($controlsHandle) }
            Test-SeparatePanelResize
        } finally {
            try { Save-PanelFocusObservation } catch { Write-Output 'Could not save the controlled-only focus observation.' }
        }
        if (-not $observation.controlledResize.completed) { throw 'The controlled-only resize did not complete its required native input.' }
        if (-not $observation.controlledResize.arrowOpenedMenu -or $observation.controlledResize.focusAfterResize -cne 'target') {
            throw 'The controlled resize did not preserve target-field focus and its first physical ArrowDown reopening.'
        }
        Write-Output 'Controlled resize passed: persisted fa, owned target search, one native 12px resize, target focus and first physical ArrowDown.'
        return
    }

    try {
    $initialCodes = Wait-PanelLanguageState 'initial-preferences-readable' { Read-PanelLanguageCodes }
    if ($initialCodes.source -cne 'en' -or $initialCodes.target -cne 'zh_tw') {
        throw "The native language fixture must begin with en and zh_tw (locale=$($Labels.locale) source=$($initialCodes.source) target=$($initialCodes.target))."
    }
    $island = Use-OverlayControl $Labels.openControls 'listening' 'Locate the language island' -Surface $controlsHandle -NameSuffix -BoundsOnly -WaitMs 5000
    Move-OverControlBounds $controlsHandle $island 'Reveal the native language island'
    Start-Sleep -Milliseconds 240
    Use-OverlayControl $Labels.openControls 'listening' 'Open the native language panel' -Surface $controlsHandle -NameSuffix -WaitMs 5000
    Use-OverlayControl $Labels.moreSettings 'listening' 'Wait for the native language panel' -Surface $controlsHandle -WaitOnly -WaitMs 5000
    $null = Wait-PanelLanguageState 'panel-focus' { [MimiPanelKeyboard]::OwnsFocus($controlsHandle) }
    $visibleValues = Save-PanelLanguageEvidence $controlsHandle $Labels $EvidencePath

    foreach ($kind in @('source', 'target')) {
        $name = $Labels.$kind
        $valueKey = $kind + 'Value'
        $initialValue = $visibleValues.$kind
        if ([string]::IsNullOrEmpty($initialValue)) { throw "The native closed language field has no observed value (field=$kind)." }
        $fixedValue = $Labels.$valueKey
        if (-not [string]::IsNullOrEmpty($fixedValue) -and -not [string]::Equals($initialValue, $fixedValue, [StringComparison]::Ordinal)) {
            throw "The native language field does not match its fixed localized label (field=$kind)."
        }
        $initialCode = $initialCodes.$kind
        $field = Wait-PanelLanguageState "$kind-field" { Get-PanelLanguageField $name }
        $bounds = $field.Current.BoundingRectangle
        if ($bounds.IsEmpty -or $bounds.Width -lt 24 -or $bounds.Height -lt 24) { throw 'A native language field has no complete click target.' }
        [MimiPointerSmoke]::ClickOverlay($controlsHandle, [int]($bounds.X + $bounds.Width / 2), [int]($bounds.Y + $bounds.Height / 2), 'Open a native language field')
        $menu = Wait-PanelLanguageState "$kind-initial-menu" { Read-PanelLanguageMenu $name }
        if (-not [string]::Equals($menu.Selected, $initialValue, [StringComparison]::Ordinal)) { throw "The native language field has the wrong initial selection (field=$kind)." }
        if ($kind -eq 'target') { Save-PanelLanguageScreenshot $controlsHandle $EvidencePath ($Labels.locale + '-target-menu.png') }
        $initialIndex = $menu.Index
        $names = $menu.Names
        $previousCode = $initialCode
        $previousName = $initialValue

        foreach ($boundary in @('Home', 'End', 'Restore')) {
            if ($boundary -ne 'Home') {
                [MimiPanelKeyboard]::Key($controlsHandle, 0x28) # ArrowDown: reopen without a mouse.
                $null = Wait-PanelLanguageState "$kind-reopen-$boundary" { Read-PanelLanguageMenu $name }
            }
            $expectedIndex = if ($boundary -eq 'Home') { 0 } elseif ($boundary -eq 'End') { $names.Count - 1 } else { $initialIndex }
            $expectedName = $names[$expectedIndex]
            [MimiPanelKeyboard]::Key($controlsHandle, $(if ($boundary -eq 'End') { 0x23 } else { 0x24 }))
            if ($boundary -eq 'Restore') {
                for ($index = 0; $index -lt $initialIndex; $index++) { [MimiPanelKeyboard]::Key($controlsHandle, 0x28) }
            }
            [MimiPanelKeyboard]::Key($controlsHandle, 0x0D) # Enter commits the actual keyboard cursor.
            $savedCodes = Wait-PanelLanguageState "$kind-save-$boundary" {
                $codes = Read-PanelLanguageCodes
                $code = if ($null -ne $codes) { $codes.$kind } else { $null }
                if ($codes -and (Test-PanelLanguageClosed $name) -and
                    ($expectedName -ceq $previousName -or $code -ne $previousCode) -and
                    ($boundary -ne 'Restore' -or $code -eq $initialCode)) { $codes }
            }
            $previousCode = $savedCodes.$kind
            $previousName = $expectedName
            if ($kind -eq 'target' -and $boundary -eq 'End') {
                $observation.active = $true
                [MimiPanelKeyboard]::ObserveNextKey()
            }
            [MimiPanelKeyboard]::Key($controlsHandle, 0x28)
            $null = Wait-PanelLanguageState "$kind-selected-$boundary" {
                $current = Read-PanelLanguageMenu $name
                if ($current -and $current.Selected -ceq $expectedName) { $current }
            }
            if ($kind -eq 'target' -and $boundary -eq 'End') {
                # Only after the original single-key selected-option assertion passes.
                try { $observation.firstReopen = Read-PanelFocusObservation 'target-selected-End-passed' }
                catch { $observation.firstReopenSnapshotFailure = $_.Exception.GetType().Name }
                $observation.active = $false
            }
            [MimiPanelKeyboard]::Key($controlsHandle, 0x1B) # Escape closes only the menu.
            $null = Wait-PanelLanguageState "$kind-escape-$boundary" { Test-PanelLanguageClosed $name }
            Use-OverlayControl $Labels.moreSettings 'listening' 'Escape preserved the panel' -Surface $controlsHandle -WaitOnly -WaitMs 5000
            Write-Output "Native panel keyboard locale=$($Labels.locale) field=$kind boundary=$boundary selected_index=$expectedIndex selected_code=$previousCode focus=owned."
        }
    }
    Assert-SettingsHidden
    $finalCodes = Wait-PanelLanguageState 'final-preferences-readable' { Read-PanelLanguageCodes }
    if ($finalCodes.source -cne 'en' -or $finalCodes.target -cne 'zh_tw') {
        throw "The native language check did not restore its initial language pair (locale=$($Labels.locale) source=$($finalCodes.source) target=$($finalCodes.target))."
    }
    Write-Output "Native panel language passed locale=$($Labels.locale): physical selection, trigger refocus, keyboard reopen and Escape; Settings hidden."
    $observation.baselinePassed = $true
    } catch {
        $observation.originalFailureType = $_.Exception.GetType().Name
        throw
    } finally {
        # Preserve the untouched baseline, including its error, before separate input.
        $observation.active = $false
        try { $null = Read-PanelFocusObservation 'baseline-final' }
        catch { $observation.finalSnapshotFailure = $_.Exception.GetType().Name }
        try { Save-PanelFocusObservation } catch { Write-Output 'Could not save the original focus observation.' }
        Test-SeparatePanelResize
        try { Save-PanelFocusObservation } catch { Write-Output 'Could not save the separate resize observation.' }
    }
}
