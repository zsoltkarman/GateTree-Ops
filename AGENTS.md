# Release notes

- Whenever you fix a bug or change user-visible behavior, update `RELEASE_NOTES.md` in the same change.
- Describe the concrete fixes and behavior changes since the previous release. Remove older released items instead of repeating the product feature list.
- Keep unreleased fixes together until the next release. Do not claim a fix is included in an already built package unless its source commit contains that fix.
- Release packaging freezes the notes with the assets. Upload retries must use those packaged notes, not the current checkout's notes.
