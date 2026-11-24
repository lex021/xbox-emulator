# Scarlett startup verification

The Windows GTX 1070 Ti run reached presentation at 3840×2160 and opened the UI texture database. All 13 captured frames were black. The title exited with 0xC0000005 in nvwgf2umx.dll at RVA 0x20a8a0. The captured exception stack includes D3D12Core.dll and TomBox before returning to the title.

The XG tiling query succeeded during real UI texture loading. Zero-address placements no longer trigger uploads from host address zero. The crash persisted after both changes.

The filtered compatibility suite passed 176 tests and three DLL-level checks of the XG slot 10 ABI. Two shared-heap migration tests also failed on the comparison build before the tile guards. The XG unit-test target requires a missing oracle module in this checkout.

A startup integration test should bring its own authorized title files locally. It should check texture loading, driver errors, a nonblack frame, and an identifiable menu before marking a run successful. Public CI can exercise the same ABI and resource paths with synthetic fixtures.

## After the upstream sync

The post-sync GTX run reached UI texture streaming and reproduced the crash. The exception stack and recorded arguments identify CopyDescriptorsSimple on a Scarlett shader-visible source heap. The next startup wave routes that heap's internal copies through the descriptor decoder and retains the guest records. This path is still being verified.

Treat these as separate startup criteria: process reaches UI resource loading, frames contain visible content, the rendered screen is identifiable as the main menu, and the audio output contains audible menu sound. A present count alone does not pass the visual check. Use locally supplied title files for end-to-end runs and synthetic fixtures in public CI.

The first descriptor-copy wave stayed alive past the previous NVIDIA exception and reached later pixel and mesh pipeline compilation. The captured frames were still black. The next wave validates root table sizes against the serialized RTS0 data, which allows the 183–261-range tables observed at startup. A synthetic test covers valid and truncated tables in both supported record sizes.

The full D3D12 unit process still exits with an access violation in the predication test group. The targeted root, descriptor-heap and pipeline suite passed 41 tests. Menu audio has not been verified.
