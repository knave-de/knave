# Compositor overview panes

The shell previously fetched full-output PNGs for every visible workspace card. GPU readback, PNG encoding, transport, decoding, and upload delayed opening and browsing.

The shell now sends at most three logical-output `OverviewPane` rectangles through `set_overview_panes`. Villain validates and stages the whole list for the next Overview surface commit, then renders existing Wayland window textures into those rectangles during the normal output frame. A pane is display-only: the overlay layer shell surface retains pointer and keyboard input, and window activation still uses existing desktop commands. The shell waits for acknowledgement before submitting the matching buffer. Requests require a registered Overview surface, allowing layout before its first mapped buffer. An empty list stages clearing; destroying or unmapping the `knave-shell-overview` surface clears it too. No persistent configuration or screenshot cache is introduced.

Knave, Villain, and Shell use the same pane request and snapshot shapes. Update
the three packages together. The PNG preview query remains available to the
non-application overview host. The direct render path uses the same bounded
workspace layout on Winit and TTY; GPU/Wayland behavior requires live smoke
verification.

Backdrop visibility follows the actual mapped Overview surface, so its wallpaper,
UI and pane geometry first appear in the same frame. Hiding and destruction clear
pending and active panes. Deploy matching Shell and Villain; rollback must restore
both because pane timing changed, although the JSON shapes and schema are unchanged.
