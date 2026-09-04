// Work in original-image pixels; CSS only scales the canvas for display.
async function editCrop(candidate, token) {
  const dialog = document.getElementById('crop-dialog');
  const canvas = document.getElementById('crop-canvas');
  const slider = document.getElementById('crop-size');
  const status = document.getElementById('crop-status');
  const ctx = canvas.getContext('2d');
  const response = await fetch(candidate.original, { headers: { 'X-Review-Token': token } });
  if (!response.ok) throw new Error('Could not load the original photo');
  const url = URL.createObjectURL(await response.blob());
  const image = new Image();
  let crop = { ...candidate.framing.crop }, drag = null;
  try {
    image.src = url;
    await image.decode();
    const width = image.naturalWidth, height = image.naturalHeight;
    if (width !== candidate.framing.width || height !== candidate.framing.height) {
      throw new Error('Original photo dimensions changed; reload before cropping');
    }
    canvas.width = width;
    canvas.height = height;
    slider.min = 96;
    slider.max = Math.min(width, height);
    function draw() {
      crop.size = Math.round(Math.max(96, Math.min(crop.size, width, height)));
      crop.x = Math.round(Math.max(0, Math.min(crop.x, width - crop.size)));
      crop.y = Math.round(Math.max(0, Math.min(crop.y, height - crop.size)));
      ctx.clearRect(0, 0, width, height);
      ctx.drawImage(image, 0, 0);
      ctx.fillStyle = 'rgba(0,0,0,.55)';
      ctx.fillRect(0, 0, width, height);
      ctx.drawImage(image, crop.x, crop.y, crop.size, crop.size, crop.x, crop.y, crop.size, crop.size);
      ctx.strokeStyle = '#fff';
      ctx.lineWidth = Math.max(2, width / 350);
      ctx.strokeRect(crop.x, crop.y, crop.size, crop.size);
      slider.value = crop.size;
      status.textContent = `${crop.size} × ${crop.size} pixels · position ${crop.x}, ${crop.y}`;
    }
    function point(event) {
      const rect = canvas.getBoundingClientRect();
      return { x: (event.clientX - rect.left) * width / rect.width,
        y: (event.clientY - rect.top) * height / rect.height };
    }
    canvas.onpointerdown = event => {
      if (event.button !== 0) return;
      event.preventDefault();
      canvas.focus();
      const p = point(event);
      const inside = p.x >= crop.x && p.x <= crop.x + crop.size && p.y >= crop.y && p.y <= crop.y + crop.size;
      drag = { x: inside ? p.x - crop.x : crop.size / 2, y: inside ? p.y - crop.y : crop.size / 2 };
      canvas.setPointerCapture(event.pointerId);
      crop.x = p.x - drag.x;
      crop.y = p.y - drag.y;
      draw();
    };
    canvas.onpointermove = event => {
      if (!drag) return;
      const p = point(event);
      crop.x = p.x - drag.x;
      crop.y = p.y - drag.y;
      draw();
    };
    canvas.onpointerup = canvas.onpointercancel = canvas.onlostpointercapture = () => { drag = null; };
    canvas.onkeydown = event => {
      const delta = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] }[event.key];
      if (!delta) return;
      event.preventDefault();
      const step = event.shiftKey ? 10 : 1;
      crop.x += delta[0] * step;
      crop.y += delta[1] * step;
      draw();
    };
    slider.oninput = () => {
      const size = Number(slider.value);
      crop.x += (crop.size - size) / 2;
      crop.y += (crop.size - size) / 2;
      crop.size = size;
      draw();
    };
    dialog.querySelectorAll('button').forEach(button => { button.disabled = false; });
    document.getElementById('crop-reset').onclick = () => { crop = { ...candidate.framing.automatic }; draw(); };
    draw();
    dialog.showModal();
    canvas.focus();
    return await new Promise(resolve => {
      document.getElementById('crop-apply').onclick = () => resolve({ ...crop });
      document.getElementById('crop-cancel').onclick = () => resolve(null);
      dialog.oncancel = event => { event.preventDefault(); resolve(null); };
    });
  } finally {
    dialog.close();
    URL.revokeObjectURL(url);
    ctx.clearRect(0, 0, canvas.width, canvas.height);
    canvas.onpointerdown = canvas.onpointermove = canvas.onpointerup = canvas.onpointercancel = null;
    canvas.onlostpointercapture = canvas.onkeydown = null;
    slider.oninput = dialog.oncancel = null;
    dialog.querySelectorAll('button').forEach(button => { button.onclick = null; });
  }
}
