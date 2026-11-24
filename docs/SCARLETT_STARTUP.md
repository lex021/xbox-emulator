# Scarlett startup verification

The Windows GTX 1070 Ti run reached presentation at 3840×2160 and opened the UI texture database. All 13 captured frames were black. The title exited with 0xC0000005 in nvwgf2umx.dll at RVA 0x20a8a0. The captured exception stack includes D3D12Core.dll and TomBox before returning to the title.

The XG tiling query succeeded during real UI texture loading. Zero-address placements no longer trigger uploads from host address zero. The crash persisted after both changes.

The filtered compatibility suite passed 176 tests and three DLL-level checks of the XG slot 10 ABI. Two shared-heap migration tests also failed on the comparison build before the tile guards. The XG unit-test target requires a missing oracle module in this checkout.

A startup integration test should bring its own authorized title files locally. It should check texture loading, driver errors, a nonblack frame, and an identifiable menu before marking a run successful. Public CI can exercise the same ABI and resource paths with synthetic fixtures.
