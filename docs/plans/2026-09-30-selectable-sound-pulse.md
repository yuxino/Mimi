# Selectable status light and shorter help copy

The user explicitly chose to keep both approved sound-shaped directions from
`d72d5be917f9b7e58719e53ec30e2d743dc94305`, rather than select only one. Integrate
its SVG/CSS shapes as `syllable` (A) and `ribbon` (B) while retaining the exact
previous integration light as `classic`, the default for old/missing preferences.
All choices express session phase only, never measured amplitude or audio data.

`pulse_style` is an ordinary persisted preference; IPC uses `pulseStyle`.
Old or unknown style values resolve to classic without replacing other fields.
Saving a style must preserve the two independent nullable animation preferences,
font and provider configuration. It uses the existing serialized settings save,
snapshot broadcast and rollback flow; it is allowed during an active session
without restarting that session. The settings preview and both overlay variants
use this same setting and existing independent reduced-motion resolution.
The 18/40px frames, persistent tracks across phases and settling pause are kept.

The two redundant help lines below diagnostic buttons move inside optional
details. Buttons, error feedback and the actual content-free diagnostic whitelist
remain. Only the now-outdated description of a breathing dot/ripples is removed;
the system preference behavior remains stated.

Android caption waiting now reads “正在等待翻译” / “Waiting for translation” /
“翻訳を待っています”, without describing internal acceptance rules. Existing
first-run proof is unchanged. A sharing-ended message and reopen action appear
only for a stopped projection/permission-lock invalidation, not every idle or
unrelated failure. Reopen uses the existing gated permission/start flow.
The successful-caption line is shorter; optional failure/configuration/billing
instructions are retained.

## Validation and visual evidence boundaries

Backend tests cover legacy/unknown style migration with existing choices,
all three styles across actual temporary-file reload and unrelated updates,
public camelCase payload and runtime-safe mutation. Frontend tests cover merge
without changing motion choices, persistent track nodes across phases,
pause/resume/off and exact frames. Existing snapshot race and reduced-motion
tests remain applicable. Android tests keep true completion proofs and distinguish
sharing recovery from fresh/active/unrelated-error states. An isolated Android
instrumentation screenshot fixture adds a sharing-ended state without capture.

The task15 video demonstrates A/B shapes on the existing shared browser canvas;
it is not the settings selector or final installed package. New settings,
diagnostic simplification and Android/desktop copy require fresh screenshots
at their exact implementation heads. Old #88 images stay previous-version
evidence until those are captured. Dated browser evidence does not establish
native macOS acceptance or real-provider behavior.
