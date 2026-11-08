# Output capture v1

Villain advertises the standard ext output capture source and image copy capture
v1 protocols. Sessions negotiate ABGR8888 SHM frames. A timer exists only while
frames are pending; rendering includes workspace content, layer surfaces, overview
panes and optionally the cursor. Session and pending-frame counts are capped at
eight, frames at 8 Mi pixels. Mode changes renew buffer constraints; removed
outputs stop the session. Cursor-only capture sessions terminate as unsupported.
See Knave ADR 0005 for backend consent, resource ownership and verification.
