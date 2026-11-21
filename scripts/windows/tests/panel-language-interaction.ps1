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
        uint sent = SendInput(2, inputs, size);
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
        if ($null -eq $list -or $list.Current.IsOffscreen) { return }
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
        if ($items.Count -gt 1 -and $items.Count -le 256 -and $selected.Count -eq 1) {
            return @{ Names = $items; Index = $selected[0]; Selected = $items[$selected[0]] }
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
    if ($Labels.locale -ceq 'en') {
        . (Join-Path $PSScriptRoot 'panel-drag-interaction.ps1')
        Test-PanelDragInteraction $controlsHandle $EvidencePath
    }

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
            [MimiPanelKeyboard]::Key($controlsHandle, 0x28)
            $null = Wait-PanelLanguageState "$kind-selected-$boundary" {
                $current = Read-PanelLanguageMenu $name
                if ($current -and $current.Selected -ceq $expectedName) { $current }
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
}
