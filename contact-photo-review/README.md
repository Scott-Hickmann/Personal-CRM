# Contact Photo Review

A standalone, local web app for finding and approving photos for Apple Contacts.
It does not use or change the CRM database, daemon, or web app.

## Run

On this Mac, from the project root:

```sh
./contact-photo-review/run
```

Requires macOS, Python 3.9+, Xcode Command Line Tools, and the `Personal CRM`
code-signing identity already used by `scripts/install-crm`. Set
`PERSONAL_CRM_CODESIGN_IDENTITY` to use another installed signing identity.
The launcher compiles and signs the small Contacts helper when needed, then
opens a browser. Allow Contacts access when prompted; access can be managed in
System Settings → Privacy & Security → Contacts. Only contacts macOS grants
access to are available. Stop with Ctrl-C. Review progress persists across runs.

To explore without touching real contacts:

```sh
./contact-photo-review/run --demo
```

Optional flags: `--no-open` and `--port 8765`. Only one instance per data folder
can run. Use the complete session link printed in the terminal when reopening.

## Review

1. Click **Load contacts**. Only cards without photos enter the review queue.
2. A background crawler discovers candidates for the entire queue, searching
   Bing with each person's name and organization. Photos download on selection.
3. Review the automatically cropped face and its linked source page. Cropping runs
   locally using Apple Vision, producing a tight square with space for hair and
   ears. Photos with no detected face, multiple faces, or a face too small for a
   clear crop are skipped automatically. Names alone are not proof of identity;
   face detection frames the photo but does not identify the person.
4. Optionally choose **Adjust crop** to see the full photo with the current square
   selected. Drag it (or use arrow keys), adjust the size slider, and choose
   **Use this crop**. **Reset to automatic** restores the initial framing; **Cancel**
   leaves the reviewed crop unchanged. Applying a crop never saves to Contacts.
5. **No, find another** permanently rejects that candidate and fetches the next.
   Identical rejected image bytes at different URLs are also skipped.
6. **Yes, save photo** backs up that contact, adds the exact reviewed image through
   Apple's Contacts framework, then moves to the next contact. The saved crop is
   exactly the one you reviewed; older cached photos are reprocessed before review.
7. Refine a search with a company or other useful context, or skip a contact.
   **Bring back skipped contacts** resumes skipped cards. **Refresh contacts**
   reconciles the queue with Apple Contacts and retries background discovery.

Public image search can return incorrect matches, repeat results, block automated
requests, or change its HTML. The app reports failures instead of guessing.
It searches up to ten pages per query; a refined query resets pagination.
Some people have no publicly available photo. Search terms go to Bing and image
hosts receive download requests; email addresses and phone numbers are not
included in automatic searches. Source-page links open directly in your browser.

## Safe saves and local data

- Uses concrete Apple contact identifiers rather than name matching or unified
  saves that might propagate to linked cards. Linked cards can appear separately.
- Immediately before saving, re-fetches the card, checks its identifying details,
  and refuses to replace an existing photo. Only `imageData` is assigned on the
  fetched card; other fields are preserved. Apple Contacts does not offer an
  atomic conditional save, so avoid editing that same card during approval.
- Saves a pre-change vCard before updating. A failed backup prevents the update.
  Checks the image checksum and verifies that a photo exists after saving.
- Records an approval intent before the native save. If a save errors or is
  interrupted, its outcome may be uncertain; refresh and inspect Contacts.
  Retrying still cannot overwrite an existing photo.
- Restricts downloads to public HTTP(S) hosts, validates redirect destinations,
  caps bytes and dimensions, and converts to a metadata-free JPEG up to 1024 px.
- The server binds only to `127.0.0.1`, checks API origin/host and a random session
  token, and never loads remote images directly in the review page.
- Private state lives in ignored `.local/review/`: `review.sqlite3`, `images/`,
  `originals/`, and `backups/`. Originals preserve the full framing, are rotated
  upright, stripped of metadata, and resized to at most 2048 pixels. Every manual
  crop uses this original instead of repeatedly cropping a crop. Demo data is
  isolated in `.local/demo/`. New files are accessible
  only to your macOS user. Backups contain contact details; treat them accordingly.

To reverse a mistaken choice, remove the photo from that card in Apple Contacts.
The vCard is a recovery reference; importing it can create a duplicate, so do not
blindly re-import it. If you want to review the card again, refresh after removing
its photo. Approved image files remain available in the local cache.

## Verify

```sh
cd contact-photo-review
python3 -m unittest -v
swiftc -parse-as-library -D PHOTO_REVIEW_TESTS contacts.swift face_crop.swift test_contacts.swift \
  -o .local/native-tests -framework Contacts -framework Vision -framework ImageIO -framework UniformTypeIdentifiers
.local/native-tests
```

Tests use fake contacts and cover rejection persistence, exact-photo approvals,
stale/cross-contact submissions, failed saves, queue reconciliation, pagination,
rejected-image deduplication, API authorization, and unsafe download destinations.
Crop tests cover square framing, coordinate conversion, edge placement, ambiguous
faces, cache upgrades, and refusing approval from a stale crop preview.
Manual-crop tests also cover original preservation, bounds validation, resetting,
and approval of the exact adjusted image.
`--demo` exercises the UI without requesting Contacts access or saving real photos.
