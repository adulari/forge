Measurement harness for the GPUI spike (all off-screen on a private Xvfb :87, mock daemon in an isolated HOME).
Scripts assume `~/.cache/forge-spike-gpui` as the working dir: `setup.sh` seeds the mock daemon (port 7611) + fixture repo,
`runall.sh` runs 3 rounds of baseline (Tauri/RN-web) and GPUI measurements. Spike env: SPIKE_VIEW, SPIKE_DIR (frame dump), SPIKE_STREAM, SPIKE_SIZE, SPIKE_ANIM.
Lavapipe (software Vulkan) was extracted from the Arch `vulkan-swrast` package into `lvp/` for the software-rendering comparison.
