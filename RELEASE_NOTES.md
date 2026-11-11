GateTree-Ops

- Release packaging now reports DMG mount failures instead of stopping silently after creating the installer.
- New releases require updated release notes describing the fixes and changes in that release.
- Resuming an upload uses the notes saved with that package, so later source changes cannot replace its release notes.

The GUI and Agent update together. Finish or stop active deployments and close SSH sessions before installing an update.
