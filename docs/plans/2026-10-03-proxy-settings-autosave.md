# Proxy settings auto-save

System and direct routes save as soon as selected, without a persistent success
message. Selecting custom opens its address field; a valid address saves on
blur, Enter or the field's explicit paste action. Typing stays local until one
of those commit actions. No connection probe accompanies a preference save.

Keep the existing active-session lock and single in-flight save guard. Failed
saves retain their selected route and address draft across optimistic rollback,
show only sanitized errors and offer an explicit retry. Successful saves track
their acknowledged configuration so Enter followed by blur does not repeat a
write while the parent snapshot catches up.

Place the custom address in the same right-aligned control column as the mode,
with a readable URL font and a compact, separated paste action. Stack the field
on narrow windows. Reuse the current neutral theme, help tooltip and native
development bundle for verification.
