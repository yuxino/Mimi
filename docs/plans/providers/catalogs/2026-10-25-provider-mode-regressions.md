# Provider configuration and asynchronous state regressions

## Scope

The audit starts from `507876b4`. It fixes reproduced failures in provider
configuration, recovery and asynchronous UI completion. It adds no provider,
changes no recording defaults and does not authorize release or live capture.

## Failures and decisions

- Raw recognition transport labels reached the desktop retry classifier without
  normalization. Normalize at the shared classifier so desktop and Android use
  the same policy. Authentication, configuration, protocol and unknown failures
  remain terminal.
- An Azure ASR profile without a translation deployment could switch to
  simultaneous mode, but saving the missing deployment required re-entering its
  existing key. Permit an unchanged saved key in the editor and resolve it in
  the backend only for the same canonical Azure resource. A changed resource or
  another profile cannot inherit the key. An otherwise valid ASR credential
  missing only its translation deployment is incomplete configuration, not a
  credential-file read failure; corrupt or inaccessible data stays unavailable.
- Android compared raw text-service endpoint strings to decide whether to reuse
  a saved optional key. An equivalent base URL could therefore erase the key.
  Use the shared protocol parsers through a credential-free JNI comparison;
  preserve explicit replacement/removal and reject invalid endpoint identities.
- A failed legacy xAI credential migration could return an error after the mode
  and normalized preferences were already saved. Prepare and verify required
  credential migration before publishing the mode, keeping successful durable
  copies available for retry without publishing an incomplete configuration.
- Creation returned unrelated global state when a newer settings event arrived,
  and the editor selected the first unfamiliar profile. Preserve the command's
  own acknowledgment and select its appended profile, as duplication already
  does, without overwriting a newer global snapshot.
- Pending immersive/lock saves could dismiss a new floating panel after the old
  panel closed. Limit completion feedback and dismissal to the mounted owner;
  the underlying preference save may still complete normally.

## Verification

Each defect first gets a failing regression, then the same case must pass after
the fix. Cover unchanged and changed endpoint identities, explicit key removal,
migration write failure and retry, raw/normalized error classification, newer
settings events and close/reopen while an action is pending.

Run affected Rust, React and actual JNI tests, followed by the canonical check
and desktop/Android CI. Inspect the signed native UI with synthetic fixtures.
Keep automated interleaving evidence separate from native layout, real provider
quality and capture behavior. Preserve the installed production application and
user data; merge only the reviewed, verified task changes.
