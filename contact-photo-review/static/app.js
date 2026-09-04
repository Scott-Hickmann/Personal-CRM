const $ = id => document.getElementById(id);
const token = location.hash.slice(1) || sessionStorage.getItem('review-token') || '';
sessionStorage.setItem('review-token', token);
history.replaceState(null, '', '/');
let contacts = [], current = null, candidate = null, busy = false, photoUrl = null, cropEditor = null;

async function api(path, data) {
  const response = await fetch(path, { method: data ? 'POST' : 'GET',
    headers: { 'X-Review-Token': token, ...(data ? { 'Content-Type': 'application/json' } : {}) },
    body: data ? JSON.stringify(data) : undefined });
  const result = await response.json();
  if (!response.ok) throw new Error(result.error || 'Request failed');
  return result;
}
function message(text, error = false) {
  $('message').textContent = text;
  $('message').classList.toggle('error', error);
}
function updateControls() {
  document.querySelectorAll('button').forEach(button => { button.disabled = busy; });
  const ready = !busy && !!candidate && !!cropEditor;
  $('no').disabled = !ready;
  $('yes').disabled = !ready || cropEditor.dirty;
  $('crop-reset').disabled = $('crop-size').disabled = !ready;
  $('crop-undo').disabled = $('crop-apply').disabled = !ready || !cropEditor.dirty;
  cropEditor?.setEnabled(ready);
}
async function work(task) {
  if (busy) return;
  busy = true;
  updateControls();
  try { await task(); } catch (error) { message(error.message, true); }
  finally { busy = false; updateControls(); }
}
function renderPeople() {
  const filter = $('filter').value.toLowerCase();
  $('people').replaceChildren();
  for (const person of contacts.filter(p => p.status === 'pending' && p.name.toLowerCase().includes(filter))) {
    const button = document.createElement('button');
    button.textContent = person.name || 'Unnamed contact';
    button.disabled = busy;
    button.classList.toggle('active', person.id === current?.id);
    const detail = document.createElement('small');
    detail.textContent = person.organization || person.email || 'Apple Contacts';
    button.append(detail);
    button.onclick = () => work(() => select(person));
    $('people').append(button);
  }
}
async function queue() {
  const result = await api('/api/queue');
  contacts = result.contacts;
  $('remaining').textContent = contacts.length;
  $('saved').textContent = result.saved;
  $('demo').hidden = !result.demo;
  $('crawl').textContent = result.crawl;
  renderPeople();
}
function clearPhoto() {
  candidate = null;
  cropEditor?.destroy();
  cropEditor = null;
  $('crop-editor').hidden = true;
  updateControls();
  if (photoUrl) URL.revokeObjectURL(photoUrl);
  photoUrl = null;
  $('source').hidden = true;
  $('source').removeAttribute('href');
  $('placeholder').hidden = false;
  $('placeholder').textContent = 'Searching for a candidate…';
  $('more').hidden = true;
}
async function showPhoto(result) {
  cropEditor?.destroy();
  cropEditor = null;
  if (photoUrl) URL.revokeObjectURL(photoUrl);
  const response = await fetch(result.image, { headers: { 'X-Review-Token': token } });
  if (!response.ok) throw new Error('Could not load the candidate photo');
  photoUrl = URL.createObjectURL(await response.blob());
  const preview = new Image();
  preview.src = photoUrl;
  await preview.decode();
  cropEditor = await createCropEditor(result, token, preview, dirty => {
    updateControls();
    message(dirty ? 'Apply your crop changes before saving this photo.' : 'Review the selected crop before saving.');
  });
  $('crop-editor').hidden = false;
  $('placeholder').hidden = true;
  const source = new URL(result.source);
  if (!['https:', 'http:'].includes(source.protocol)) throw new Error('Invalid source link');
  $('source').href = source.href;
  $('source').textContent = result.title || source.hostname;
  $('source').hidden = false;
  $('query').value = result.query;
  candidate = result;
}
async function find(query) {
  clearPhoto();
  message(`Searching the web for ${current.name}…`);
  try {
    const result = await api('/api/candidate', { person: current.id, ...(query ? { query } : {}) });
    await showPhoto(result);
    message('Candidate ready. Confirm the person using the photo and source page.');
  } catch (error) {
    $('placeholder').textContent = 'No photo ready. Try more candidates or refine your search above.';
    $('more').hidden = false;
    throw error;
  }
}
async function select(person) {
  current = person;
  $('empty').hidden = true;
  $('review').hidden = false;
  $('name').textContent = person.name || 'Unnamed contact';
  $('details').textContent = [person.job, person.organization, person.email].filter(Boolean).join(' · ');
  $('query').value = person.query;
  renderPeople();
  await find();
}
async function next() {
  clearPhoto();
  current = null;
  await queue();
  const person = contacts.find(p => p.status === 'pending');
  if (person) await select(person);
  else {
    $('review').hidden = true;
    $('empty').hidden = false;
    message(contacts.length ? 'Remaining contacts are skipped. Bring them back whenever you’re ready.' : 'All available contacts are reviewed. Refresh to check for more.');
  }
}
const refresh = () => work(async () => {
  clearPhoto();
  message('Reading Apple Contacts… Allow access if macOS asks.');
  await api('/api/refresh', {});
  await next();
});
$('start').onclick = $('refresh').onclick = refresh;
$('filter').oninput = renderPeople;
$('search').onsubmit = event => { event.preventDefault(); work(() => find($('query').value)); };
$('more').onclick = () => work(() => find());
$('no').onclick = () => work(async () => {
  await api('/api/decide', { person: current.id, candidate: candidate.id, approved: false });
  await find();
});
$('yes').onclick = () => work(async () => {
  message('Backing up the contact and saving your approved photo…');
  await api('/api/decide', { person: current.id, candidate: candidate.id, approved: true, sha256: candidate.sha256 });
  await next();
});
$('crop-reset').onclick = () => cropEditor?.reset();
$('crop-undo').onclick = () => cropEditor?.undo();
$('crop-apply').onclick = () => work(async () => {
  const selected = candidate;
  const crop = cropEditor.crop;
  candidate = null;
  message('Preparing your adjusted crop…');
  const result = await api('/api/recrop', { person: current.id, candidate: selected.id, sha256: selected.sha256, crop });
  await showPhoto(result);
  message('Crop updated. Review it before saving the photo to Contacts.');
});
$('skip').onclick = () => work(async () => { await api('/api/skip', { person: current.id }); await next(); });
$('resume').onclick = () => work(async () => { await api('/api/resume', {}); await next(); });
work(async () => {
  await queue();
  if (contacts.some(p => p.status === 'pending')) await next();
});
setInterval(async () => {
  if (!busy) {
    try { $('crawl').textContent = (await api('/api/queue')).crawl; } catch (_) { /* Explicit actions report failures. */ }
  }
}, 5000);
