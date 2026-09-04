// Work in original-image pixels; CSS only scales the canvas for display.
async function createCropEditor(candidate, token, preview, onChange) {
  const canvas = document.getElementById('crop-canvas');
  const slider = document.getElementById('crop-size');
  const status = document.getElementById('crop-status');
  const ctx = canvas.getContext('2d');
  const response = await fetch(candidate.original, { headers: { 'X-Review-Token': token } });
  if (!response.ok) throw new Error('Could not load the original photo');
  const url = URL.createObjectURL(await response.blob());
  const image = new Image();
  let crop = { ...candidate.framing.crop }, drag = null, enabled = false;
  const dirty = () => ['x', 'y', 'size'].some(key => crop[key] !== candidate.framing.crop[key]);
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
      if (dirty()) {
        ctx.drawImage(image, crop.x, crop.y, crop.size, crop.size, crop.x, crop.y, crop.size, crop.size);
      } else {
        ctx.drawImage(preview, crop.x, crop.y, crop.size, crop.size);
      }
      // Shade the square's corners to preview a circular Contacts display.
      ctx.beginPath();
      ctx.rect(crop.x, crop.y, crop.size, crop.size);
      ctx.moveTo(crop.x + crop.size, crop.y + crop.size / 2);
      ctx.arc(crop.x + crop.size / 2, crop.y + crop.size / 2, crop.size / 2, 0, Math.PI * 2);
      ctx.fillStyle = 'rgba(0,0,0,.22)';
      ctx.fill('evenodd');
      ctx.strokeStyle = '#fff';
      ctx.lineWidth = Math.max(2, width / 350);
      ctx.strokeRect(crop.x, crop.y, crop.size, crop.size);
      ctx.beginPath();
      ctx.arc(crop.x + crop.size / 2, crop.y + crop.size / 2, crop.size / 2, 0, Math.PI * 2);
      ctx.stroke();
      slider.value = crop.size;
      status.textContent = `${crop.size} × ${crop.size} pixels · position ${crop.x}, ${crop.y}`;
      onChange(dirty());
    }
    function point(event) {
      const rect = canvas.getBoundingClientRect();
      return { x: (event.clientX - rect.left) * width / rect.width,
        y: (event.clientY - rect.top) * height / rect.height };
    }
    canvas.onpointerdown = event => {
      if (!enabled || event.button !== 0) return;
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
      if (!enabled || !drag) return;
      const p = point(event);
      crop.x = p.x - drag.x;
      crop.y = p.y - drag.y;
      draw();
    };
    canvas.onpointerup = canvas.onpointercancel = canvas.onlostpointercapture = () => { drag = null; };
    canvas.onkeydown = event => {
      const delta = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] }[event.key];
      if (!enabled || !delta) return;
      event.preventDefault();
      const step = event.shiftKey ? 10 : 1;
      crop.x += delta[0] * step;
      crop.y += delta[1] * step;
      draw();
    };
    slider.oninput = () => {
      if (!enabled) return;
      const size = Number(slider.value);
      crop.x += (crop.size - size) / 2;
      crop.y += (crop.size - size) / 2;
      crop.size = size;
      draw();
    };
    draw();
    return {
      get dirty() { return dirty(); },
      get crop() { return { ...crop }; },
      reset() { if (enabled) { crop = { ...candidate.framing.automatic }; draw(); } },
      undo() { if (enabled) { crop = { ...candidate.framing.crop }; draw(); } },
      setEnabled(value) { enabled = value; if (!value) drag = null; },
      destroy() {
        enabled = false;
        URL.revokeObjectURL(url);
        ctx.clearRect(0, 0, canvas.width, canvas.height);
        canvas.onpointerdown = canvas.onpointermove = canvas.onpointerup = canvas.onpointercancel = null;
        canvas.onlostpointercapture = canvas.onkeydown = null;
        slider.oninput = null;
      }
    };
  } catch (error) {
    URL.revokeObjectURL(url);
    throw error;
  }
}
