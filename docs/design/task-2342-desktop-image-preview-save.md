# Task 2342: Desktop image preview save

## Goal

Let users save the full-resolution image currently shown in the Mac desktop
chat lightbox. Saving must work for both inline base64 transcript images and
workspace-path images loaded through the preview API.

## Design

- Keep `ImageZoomDialog` as the single UI entry point. Both transcript image
  paths already converge on a base64 data URL there, so the lightbox adds one
  visible download button without source-specific branches.
- Expose a typed `saveImage` method on the existing preload bridge. The
  renderer sends only the data URL and an optional suggested name; it does not
  choose or write a filesystem path.
- In the main process, validate and decode the base64 image data URL, infer a
  safe extension from its media type, and build a sanitized suggested file
  name. Show Electron's native save dialog, then write the decoded bytes to the
  path selected by the user.
- Treat cancel as a normal no-op. Report successful writes and failures through
  the existing desktop toast surface.

This preserves the frozen `window.garyxDesktop` contract: preload materializes
the new method on the bridge object, and no renderer proxy or property
substitution is introduced.

## Validation

- Unit-test media-type extension inference, suggested-name normalization, data
  URL validation, and byte-exact base64 decoding.
- Run the complete desktop unit suite and packaged build.
- In the installed app, save one inline image and one workspace-path image,
  then compare each saved file's bytes with the source payload.
