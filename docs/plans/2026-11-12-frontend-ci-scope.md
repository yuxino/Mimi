# CI scope for editorial changes and native document inputs

Keep the `ci` workflow and its scope/architecture guards running on every change.
Skip the frontend job only when the changed paths are known editorial Markdown:
ordinary development, plan, QA, localized README and usage documentation, plus
the listed root/community guides. Unknown paths continue to run the job.

Release-note Markdown, demo fixtures, debugger evidence, notices and licenses
remain frontend-required. Code, configuration, lock files, assets, scripts,
workflows and shared/Android generation inputs also run frontend checks. Full
validation, tags, requested packages and unavailable diffs force them regardless
of paths. Rename detection remains disabled so a source file moved into
documentation still counts as a source deletion.

Two exact paths under docs are also native inputs, which the previous native
scope missed. `docs/development/debugger-evidence.md` is embedded by the desktop
development debugger and compiled by desktop tests. A change must run native
checks on every supported desktop OS. There is no Android consumer for this
document, so it does not trigger Android builds.

`docs/demos/english-film/response.json` is read by the shared runtime's captured
Alibaba pairing regression. Both desktop and Android CI run that test through
`scripts/check-shared-core.sh`. A change must run desktop native checks and
trigger Android CI on pull requests and main pushes. The existing architecture
guard and rejection table enforce this fixture path in both Android events.
Do not expand either exception to all development or demo documentation.

Expose the plan through `scope.outputs.frontend`. Both the job condition and
`ci-complete` may skip frontend only for the explicit string `false`; a missing
output must retain the requirement. Failed/cancelled/skipped planned jobs and a
failed scope job must still fail the aggregate. Keep existing native platform
classification and Android desktop/shared coverage, adding only the verified
document inputs above. Release dependencies remain unchanged.

This removes unnecessary frontend work for editorial changes and closes missing
native fixture coverage. It does not change installed-app behavior or establish
a local build or runtime performance gain.
