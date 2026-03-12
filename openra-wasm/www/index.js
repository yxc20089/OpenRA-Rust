import init, { ReplayViewer, GameSession, SpriteAtlas } from './pkg/openra_wasm.js';

// ── DOM refs ──
const homeEl = document.getElementById('home');
const replaySetupEl = document.getElementById('replay-setup');
const gameUiEl = document.getElementById('game-ui');
const canvas = document.getElementById('canvas');
const ctx = canvas.getContext('2d');

const hudCash = document.getElementById('hud-cash');
const hudPower = document.getElementById('hud-power');
const hudTick = document.getElementById('hud-tick');
const hudMode = document.getElementById('hud-mode');
const hudMsg = document.getElementById('hud-msg');

const selSection = document.getElementById('sel-section');
const selInfo = document.getElementById('sel-info');
const selActions = document.getElementById('sel-actions');
const queueSection = document.getElementById('queue-section');
const queueList = document.getElementById('queue-list');
const buildingList = document.getElementById('building-list');
const unitList = document.getElementById('unit-list');
const replayControlsEl = document.getElementById('replay-controls');

// ── State ──
let mode = null;
let session = null;
let humanPlayerId = null;
let selectedUnits = [];
let lastSnapshot = null;
let buildableItems = null;
let placementMode = null;
let playing = false;
let animFrameId = null;
let mouseCell = { x: -1, y: -1 };
let atlas = null; // SpriteAtlas
let spriteImages = {}; // name -> [ImageBitmap per frame]
let spriteInfo = {}; // name -> { width, height, frames }

// Rendering geometry
let mapW = 128, mapH = 128, scale = 1, offsetX = 0, offsetY = 0;

// ── Isometric constants ──
// Each cell is rendered as a diamond. Cell size in pixels at scale=1:
const ISO_W = 48; // diamond width
const ISO_H = 24; // diamond height

// ── Player colors with gradients ──
const PLAYER_COLORS = {
    0: { main: '#666', light: '#888', dark: '#444' },
    1: { main: '#666', light: '#888', dark: '#444' },
    2: { main: '#666', light: '#888', dark: '#444' },
    3: { main: '#3377dd', light: '#55aaff', dark: '#2255aa' },
    4: { main: '#dd3344', light: '#ff5566', dark: '#aa2233' },
    5: { main: '#666', light: '#888', dark: '#444' },
    6: { main: '#33aa33', light: '#55cc55', dark: '#228822' },
    7: { main: '#aa33aa', light: '#cc55cc', dark: '#882288' },
};

const BUILDING_FOOTPRINTS = {
    'fact': [3, 2], 'weap': [3, 2], 'weap.ukraine': [3, 2], 'proc': [3, 2],
    'fix': [3, 2], 'spen': [3, 3], 'syrd': [3, 3],
    'powr': [2, 2], 'apwr': [2, 2], 'tent': [2, 2], 'barr': [2, 2],
    'dome': [2, 2], 'hpad': [2, 2], 'afld': [2, 2], 'atek': [2, 2], 'stek': [2, 2],
    'tsla': [1, 1], 'sam': [2, 1], 'gap': [2, 2], 'agun': [1, 1],
    'pbox': [1, 1], 'hbox': [1, 1], 'gun': [1, 1], 'ftur': [1, 1],
};

// ── Sprite loading ──
async function loadSprites() {
    try {
        atlas = new SpriteAtlas();
        spriteInfo = JSON.parse(atlas.info_json());
        console.log('Loaded sprites:', Object.keys(spriteInfo));

        for (const [name, info] of Object.entries(spriteInfo)) {
            spriteImages[name] = [];
            // Only preload first frame and a few key ones
            const framesToLoad = Math.min(info.frames, 4);
            for (let i = 0; i < framesToLoad; i++) {
                const rgba = atlas.frame_rgba(name, i);
                if (rgba.length > 0) {
                    const imgData = new ImageData(
                        new Uint8ClampedArray(rgba),
                        info.width, info.height
                    );
                    const bmp = await createImageBitmap(imgData);
                    spriteImages[name].push(bmp);
                }
            }
        }
        console.log('Sprite images ready');
    } catch (e) {
        console.error('Sprite loading failed:', e);
    }
}

// ── Navigation ──
function showScreen(screen) {
    homeEl.style.display = screen === 'home' ? 'flex' : 'none';
    replaySetupEl.style.display = screen === 'replay-setup' ? 'flex' : 'none';
    gameUiEl.style.display = screen === 'game' ? 'block' : 'none';
}

document.getElementById('btn-start-game').addEventListener('click', startGame);
document.getElementById('btn-watch-replay').addEventListener('click', () => showScreen('replay-setup'));
document.getElementById('btn-back-home').addEventListener('click', () => showScreen('home'));

// ── Replay file loading ──
let replayBytes = null, mapBytes = null;
const replayInput = document.getElementById('replay-file');
const mapInput = document.getElementById('map-file');
const btnLoadReplay = document.getElementById('btn-load-replay');
const replayStatus = document.getElementById('replay-status');

function checkReplayFiles() { btnLoadReplay.disabled = !(replayBytes && mapBytes); }

replayInput.addEventListener('change', async (e) => {
    const f = e.target.files[0];
    if (f) { replayBytes = new Uint8Array(await f.arrayBuffer()); replayStatus.textContent = `Replay: ${f.name}`; }
    checkReplayFiles();
});
mapInput.addEventListener('change', async (e) => {
    const f = e.target.files[0];
    if (f) { mapBytes = new Uint8Array(await f.arrayBuffer()); replayStatus.textContent = `Map: ${f.name}`; }
    checkReplayFiles();
});

btnLoadReplay.addEventListener('click', () => {
    try {
        session = new ReplayViewer(replayBytes, mapBytes);
        mode = 'replay';
        humanPlayerId = null;
        selectedUnits = [];
        placementMode = null;
        playing = false;
        setupReplayUI();
        showScreen('game');
        resizeCanvas();
        lastSnapshot = JSON.parse(session.snapshot_json());
        render(lastSnapshot);
    } catch (e) { replayStatus.textContent = `Error: ${e}`; }
});

function setupReplayUI() {
    replayControlsEl.style.display = 'block';
    document.getElementById('build-buildings').style.display = 'none';
    document.getElementById('build-units').style.display = 'none';
    hudMode.textContent = `Replay: 0 / ${session.total_frames()}`;
    document.getElementById('btn-play').onclick = () => {
        if (playing) { playing = false; document.getElementById('btn-play').textContent = 'Play'; }
        else { playing = true; document.getElementById('btn-play').textContent = 'Pause'; replayLoop(); }
    };
    document.getElementById('btn-step').onclick = () => { if (session) replayStep(); };
    document.getElementById('speed').oninput = (e) => { document.getElementById('speed-val').textContent = e.target.value; };
}

function replayStep() {
    if (!session.tick()) { playing = false; hudMsg.textContent = 'Replay finished.'; return false; }
    lastSnapshot = JSON.parse(session.snapshot_json());
    render(lastSnapshot);
    hudMode.textContent = `Replay: ${session.current_frame()} / ${session.total_frames()}`;
    updateHUD(lastSnapshot);
    return true;
}

function replayLoop() {
    if (!playing || !session) return;
    const fps = parseInt(document.getElementById('speed').value);
    for (let i = 0; i < fps; i++) { if (!replayStep()) return; }
    animFrameId = requestAnimationFrame(replayLoop);
}

// ── Game start ──
function startGame() {
    try {
        session = new GameSession();
        mode = 'game';
        humanPlayerId = session.human_player_id();
        selectedUnits = [];
        placementMode = null;
        playing = true;
        PLAYER_COLORS[humanPlayerId] = { main: '#3377dd', light: '#55aaff', dark: '#2255aa' };
        PLAYER_COLORS[humanPlayerId + 1] = { main: '#dd3344', light: '#ff5566', dark: '#aa2233' };
        setupGameUI();
        showScreen('game');
        resizeCanvas();
        gameLoop();
    } catch (e) { alert(`Failed to start game: ${e}`); }
}

function setupGameUI() {
    replayControlsEl.style.display = 'none';
    document.getElementById('build-buildings').style.display = 'block';
    document.getElementById('build-units').style.display = 'block';
    hudMode.textContent = 'vs Bot';
    hudMsg.textContent = 'Deploy your MCV! Select it and click Deploy.';
    refreshBuildable();
}

function gameLoop() {
    if (!playing || mode !== 'game') return;
    const alive = session.tick();
    lastSnapshot = JSON.parse(session.snapshot_json());
    render(lastSnapshot);
    updateHUD(lastSnapshot);
    refreshQueue(lastSnapshot);
    if (!alive) {
        playing = false;
        const winner = session.winner();
        hudMsg.textContent = winner === humanPlayerId ? 'You win!' : 'You lost!';
        return;
    }
    if (session.current_frame() % 30 === 0) refreshBuildable();
    setTimeout(gameLoop, 40);
}

// ── Sidebar: buildable items ──
function refreshBuildable() {
    if (mode !== 'game') return;
    try { buildableItems = JSON.parse(session.buildable_items_json()); } catch { buildableItems = []; }
    const buildings = buildableItems.filter(i => i.is_building);
    const units = buildableItems.filter(i => !i.is_building);

    buildingList.innerHTML = '';
    for (const item of buildings) {
        const btn = document.createElement('button');
        btn.className = 'build-btn';
        btn.innerHTML = `${item.name} <span class="cost">$${item.cost}</span>`;
        btn.onclick = () => session.order_start_production(item.name);
        buildingList.appendChild(btn);
    }
    unitList.innerHTML = '';
    for (const item of units) {
        const btn = document.createElement('button');
        btn.className = 'build-btn';
        btn.innerHTML = `${item.name} <span class="cost">$${item.cost}</span>`;
        btn.onclick = () => session.order_start_production(item.name);
        unitList.appendChild(btn);
    }
}

function refreshQueue(snapshot) {
    if (mode !== 'game' || !snapshot) return;
    const myPlayer = snapshot.players.find(p => p.index === humanPlayerId);
    if (!myPlayer || !myPlayer.production_queue || myPlayer.production_queue.length === 0) {
        queueSection.style.display = 'none'; return;
    }
    queueSection.style.display = 'block';
    queueList.innerHTML = '';
    for (const item of myPlayer.production_queue) {
        const div = document.createElement('div');
        div.className = 'queue-item' + (item.done ? ' done' : '');
        const pct = Math.round(item.progress * 100);
        div.innerHTML = `${item.item_name} ${pct}%<div class="bar"><div class="bar-fill" style="width:${pct}%"></div></div>`;
        if (item.done) {
            const buildInfo = buildableItems?.find(b => b.name === item.item_name && b.is_building);
            if (buildInfo) {
                div.style.cursor = 'pointer'; div.style.color = '#44cc44';
                div.innerHTML += ' <small>(click map to place)</small>';
                div.onclick = () => {
                    placementMode = { type: item.item_name, footprint: [buildInfo.footprint[0], buildInfo.footprint[1]] };
                    hudMsg.textContent = `Place ${item.item_name} — click on map (right-click to cancel)`;
                };
                if (!placementMode) {
                    placementMode = { type: item.item_name, footprint: [buildInfo.footprint[0], buildInfo.footprint[1]] };
                    hudMsg.textContent = `Place ${item.item_name} — click on map`;
                }
            }
        }
        queueList.appendChild(div);
    }
}

function refreshSelection() {
    if (selectedUnits.length === 0 || !lastSnapshot) { selSection.style.display = 'none'; return; }
    selSection.style.display = 'block';
    const actors = lastSnapshot.actors.filter(a => selectedUnits.includes(a.id));
    if (actors.length === 0) { selSection.style.display = 'none'; return; }

    if (actors.length === 1) {
        const a = actors[0];
        const hpPct = a.max_hp > 0 ? Math.round(a.hp / a.max_hp * 100) : 100;
        selInfo.innerHTML = `<span class="name">${a.actor_type || a.kind}</span><br>HP: ${a.hp}/${a.max_hp} (${hpPct}%)<br>Activity: ${a.activity}`;
    } else {
        selInfo.innerHTML = `<span class="name">${actors.length} units selected</span>`;
    }
    selActions.innerHTML = '';
    if (actors.some(a => a.kind === 'Mcv')) {
        const btn = document.createElement('button'); btn.className = 'action-btn'; btn.textContent = 'Deploy';
        btn.onclick = () => { for (const a of actors) if (a.kind === 'Mcv') session.order_deploy(a.id); };
        selActions.appendChild(btn);
    }
    if (actors.some(a => a.kind === 'Building')) {
        const btn = document.createElement('button'); btn.className = 'action-btn'; btn.textContent = 'Sell';
        btn.onclick = () => { for (const a of actors) if (a.kind === 'Building') session.order_sell(a.id); };
        selActions.appendChild(btn);
    }
    const stopBtn = document.createElement('button'); stopBtn.className = 'action-btn'; stopBtn.textContent = 'Stop';
    stopBtn.onclick = () => { for (const a of actors) session.order_stop(a.id); };
    selActions.appendChild(stopBtn);
}

function updateHUD(snapshot) {
    if (!snapshot) return;
    const myPlayer = snapshot.players.find(p => p.index === humanPlayerId);
    if (myPlayer) {
        hudCash.textContent = `$${myPlayer.cash}`;
        const low = myPlayer.power_drained > myPlayer.power_provided;
        hudPower.textContent = `Power: ${myPlayer.power_provided}/${myPlayer.power_drained}${low ? ' LOW' : ''}`;
        hudPower.style.color = low ? '#e94560' : '#e0e0e0';
    } else if (mode === 'replay' && snapshot.players.length > 0) {
        hudCash.textContent = `P${snapshot.players[0].index}: $${snapshot.players[0].cash}`;
    }
    hudTick.textContent = `Tick ${snapshot.tick}`;
}

// ── Canvas resize ──
function resizeCanvas() {
    const wrap = document.getElementById('canvas-wrap');
    canvas.width = wrap.clientWidth;
    canvas.height = wrap.clientHeight;
}
window.addEventListener('resize', () => { resizeCanvas(); if (lastSnapshot) render(lastSnapshot); });

// ── Coordinate conversion (top-down) ──
function computeGeometry(snapshot) {
    mapW = snapshot.map_width || 128;
    mapH = snapshot.map_height || 128;
    const scaleX = canvas.width / mapW;
    const scaleY = canvas.height / mapH;
    scale = Math.min(scaleX, scaleY);
    offsetX = (canvas.width - mapW * scale) / 2;
    offsetY = (canvas.height - mapH * scale) / 2;
}

function cellToPixel(cx, cy) {
    return {
        x: offsetX + cx * scale + scale / 2,
        y: offsetY + cy * scale + scale / 2,
    };
}

function canvasToCell(px, py) {
    return {
        x: Math.floor((px - offsetX) / scale),
        y: Math.floor((py - offsetY) / scale),
    };
}

function actorAtCell(cx, cy, snapshot) {
    for (const a of snapshot.actors) {
        if (a.kind === 'Building') {
            const fp = BUILDING_FOOTPRINTS[a.actor_type] || [2, 2];
            if (cx >= a.x && cx < a.x + fp[0] && cy >= a.y && cy < a.y + fp[1]) return a;
        }
    }
    for (const a of snapshot.actors) {
        if (a.kind !== 'Building' && a.kind !== 'Tree' && a.kind !== 'Mine') {
            if (a.x === cx && a.y === cy) return a;
        }
    }
    return null;
}

// ── Mouse input ──
canvas.addEventListener('mousemove', (e) => {
    const rect = canvas.getBoundingClientRect();
    mouseCell = canvasToCell(e.clientX - rect.left, e.clientY - rect.top);
    if (placementMode && lastSnapshot) render(lastSnapshot);
});

canvas.addEventListener('click', (e) => {
    if (!lastSnapshot) return;
    const rect = canvas.getBoundingClientRect();
    const cell = canvasToCell(e.clientX - rect.left, e.clientY - rect.top);
    if (mode === 'game') handleGameClick(cell, e.shiftKey);
    else if (mode === 'replay') {
        const actor = actorAtCell(cell.x, cell.y, lastSnapshot);
        selectedUnits = actor ? [actor.id] : [];
        refreshSelection();
    }
});

canvas.addEventListener('contextmenu', (e) => {
    e.preventDefault();
    if (mode !== 'game' || !lastSnapshot) return;
    const rect = canvas.getBoundingClientRect();
    const cell = canvasToCell(e.clientX - rect.left, e.clientY - rect.top);
    if (placementMode) { placementMode = null; hudMsg.textContent = ''; return; }
    if (selectedUnits.length === 0) return;
    const target = actorAtCell(cell.x, cell.y, lastSnapshot);
    if (target && target.owner !== humanPlayerId && target.owner > 2) {
        for (const uid of selectedUnits) session.order_attack(uid, target.id);
        hudMsg.textContent = `Attacking ${target.actor_type || target.kind}`;
    } else {
        for (const uid of selectedUnits) session.order_move(uid, cell.x, cell.y);
        hudMsg.textContent = `Moving to (${cell.x}, ${cell.y})`;
    }
});

function handleGameClick(cell, shiftKey) {
    if (placementMode) {
        if (session.can_place_building(placementMode.type, cell.x, cell.y)) {
            session.order_place_building(placementMode.type, cell.x, cell.y);
            hudMsg.textContent = `Placed ${placementMode.type}`;
            placementMode = null; refreshBuildable();
        } else { hudMsg.textContent = 'Cannot place here!'; }
        return;
    }
    const actor = actorAtCell(cell.x, cell.y, lastSnapshot);
    if (actor && actor.owner === humanPlayerId) {
        if (shiftKey) { if (!selectedUnits.includes(actor.id)) selectedUnits.push(actor.id); }
        else { selectedUnits = [actor.id]; }
    } else if (actor && actor.owner !== humanPlayerId && actor.owner > 2 && selectedUnits.length > 0) {
        for (const uid of selectedUnits) session.order_attack(uid, actor.id);
        hudMsg.textContent = `Attacking ${actor.actor_type || actor.kind}`;
    } else {
        if (!shiftKey) selectedUnits = [];
    }
    refreshSelection();
}

// Drag select
let dragStart = null;
canvas.addEventListener('mousedown', (e) => {
    if (e.button !== 0) return;
    const rect = canvas.getBoundingClientRect();
    dragStart = { x: e.clientX - rect.left, y: e.clientY - rect.top };
});
canvas.addEventListener('mouseup', (e) => {
    if (e.button !== 0 || !dragStart || !lastSnapshot || mode !== 'game') { dragStart = null; return; }
    const rect = canvas.getBoundingClientRect();
    const dragEnd = { x: e.clientX - rect.left, y: e.clientY - rect.top };
    if (Math.abs(dragEnd.x - dragStart.x) > 10 || Math.abs(dragEnd.y - dragStart.y) > 10) {
        const c1 = canvasToCell(Math.min(dragStart.x, dragEnd.x), Math.min(dragStart.y, dragEnd.y));
        const c2 = canvasToCell(Math.max(dragStart.x, dragEnd.x), Math.max(dragStart.y, dragEnd.y));
        selectedUnits = [];
        for (const a of lastSnapshot.actors) {
            if (a.owner !== humanPlayerId) continue;
            if (a.kind === 'Building' || a.kind === 'Tree' || a.kind === 'Mine') continue;
            if (a.x >= c1.x && a.x <= c2.x && a.y >= c1.y && a.y <= c2.y) selectedUnits.push(a.id);
        }
        refreshSelection();
    }
    dragStart = null;
});

document.addEventListener('keydown', (e) => {
    if (mode !== 'game') return;
    if (e.key === 'Escape') {
        if (placementMode) { placementMode = null; hudMsg.textContent = ''; }
        else { selectedUnits = []; refreshSelection(); }
    }
    if (e.key === 's' || e.key === 'S') { for (const uid of selectedUnits) session.order_stop(uid); }
    if (e.key === 'd' || e.key === 'D') {
        if (!lastSnapshot) return;
        for (const uid of selectedUnits) {
            const a = lastSnapshot.actors.find(a => a.id === uid);
            if (a && a.kind === 'Mcv') session.order_deploy(uid);
        }
    }
});

// ── Color helpers ──
function getColor(owner) {
    const c = PLAYER_COLORS[owner];
    return c ? c.main : '#fff';
}
function getLightColor(owner) {
    const c = PLAYER_COLORS[owner];
    return c ? c.light : '#fff';
}
function getDarkColor(owner) {
    const c = PLAYER_COLORS[owner];
    return c ? c.dark : '#888';
}

// ── Render ──
function render(snapshot) {
    const w = canvas.width, h = canvas.height;
    computeGeometry(snapshot);
    if (!snapshot) return;

    // Terrain background - textured green
    ctx.fillStyle = '#2a4a2a';
    ctx.fillRect(0, 0, w, h);

    // Terrain texture pattern
    drawTerrain(snapshot);

    // Sort actors by y for depth (painter's algorithm)
    const sorted = [...snapshot.actors].sort((a, b) => {
        const kindOrder = { 'Tree': 0, 'Mine': 0, 'Building': 1, 'Infantry': 2, 'Vehicle': 2, 'Mcv': 2, 'Aircraft': 3, 'Ship': 2 };
        const ka = kindOrder[a.kind] ?? 2;
        const kb = kindOrder[b.kind] ?? 2;
        if (ka !== kb) return ka - kb;
        return a.y - b.y || a.x - b.x;
    });

    // Resources
    if (snapshot.resources) {
        for (const res of snapshot.resources) {
            const rx = offsetX + res.x * scale;
            const ry = offsetY + res.y * scale;
            if (res.kind === 1) {
                // Ore - golden specks
                const alpha = 0.4 + 0.06 * res.density;
                ctx.fillStyle = `rgba(180,140,40,${alpha})`;
                ctx.fillRect(rx + 1, ry + 1, scale - 2, scale - 2);
                // Add some texture dots
                ctx.fillStyle = `rgba(220,180,60,${alpha * 0.7})`;
                for (let d = 0; d < res.density; d++) {
                    const dx = (((res.x * 7 + d * 13) % 5) / 5) * (scale - 4) + 2;
                    const dy = (((res.y * 11 + d * 17) % 5) / 5) * (scale - 4) + 2;
                    ctx.fillRect(rx + dx, ry + dy, 2, 2);
                }
            } else if (res.kind === 2) {
                // Gems - sparkling purple
                const alpha = 0.5 + 0.06 * res.density;
                ctx.fillStyle = `rgba(140,40,180,${alpha})`;
                ctx.fillRect(rx + 1, ry + 1, scale - 2, scale - 2);
                ctx.fillStyle = `rgba(200,100,255,${alpha * 0.8})`;
                for (let d = 0; d < res.density; d++) {
                    const dx = (((res.x * 7 + d * 13) % 5) / 5) * (scale - 4) + 2;
                    const dy = (((res.y * 11 + d * 17) % 5) / 5) * (scale - 4) + 2;
                    ctx.beginPath();
                    ctx.arc(rx + dx + 1, ry + dy + 1, 1.5, 0, Math.PI * 2);
                    ctx.fill();
                }
            }
        }
    }

    // Render all actors (sorted by depth)
    for (const a of sorted) {
        if (a.kind === 'Tree') drawTree(a);
        else if (a.kind === 'Mine') drawMine(a);
        else if (a.kind === 'Building') drawBuilding(a);
        else drawUnit(a);
    }

    // Placement ghost
    if (placementMode && mouseCell.x >= 0) {
        drawPlacementGhost();
    }

    // Replay player info overlay
    if (mode === 'replay') drawReplayOverlay(snapshot);
}

function drawTerrain(snapshot) {
    // Subtle grid with terrain variation
    if (scale < 3) return;

    // Grid lines
    ctx.strokeStyle = 'rgba(60,90,60,0.3)';
    ctx.lineWidth = 0.5;
    for (let x = 0; x <= mapW; x++) {
        ctx.beginPath();
        ctx.moveTo(offsetX + x * scale, offsetY);
        ctx.lineTo(offsetX + x * scale, offsetY + mapH * scale);
        ctx.stroke();
    }
    for (let y = 0; y <= mapH; y++) {
        ctx.beginPath();
        ctx.moveTo(offsetX, offsetY + y * scale);
        ctx.lineTo(offsetX + mapW * scale, offsetY + y * scale);
        ctx.stroke();
    }

    // Add subtle terrain variation
    if (scale > 4) {
        for (let y = 0; y < mapH; y += 3) {
            for (let x = 0; x < mapW; x += 3) {
                const hash = (x * 7919 + y * 7727) & 0xFF;
                if (hash < 30) {
                    // Dark patches
                    ctx.fillStyle = 'rgba(20,40,20,0.15)';
                    ctx.fillRect(offsetX + x * scale, offsetY + y * scale, scale * 2, scale * 2);
                } else if (hash > 230) {
                    // Light patches
                    ctx.fillStyle = 'rgba(50,80,50,0.1)';
                    ctx.fillRect(offsetX + x * scale, offsetY + y * scale, scale * 3, scale * 2);
                }
            }
        }
    }
}

function drawTree(a) {
    const px = offsetX + a.x * scale + scale / 2;
    const py = offsetY + a.y * scale + scale / 2;
    const s = Math.max(3, scale * 0.8);

    // Shadow
    ctx.fillStyle = 'rgba(0,0,0,0.2)';
    ctx.beginPath();
    ctx.ellipse(px + s * 0.2, py + s * 0.3, s * 0.5, s * 0.25, 0, 0, Math.PI * 2);
    ctx.fill();

    // Trunk
    ctx.fillStyle = '#4a3520';
    ctx.fillRect(px - s * 0.1, py - s * 0.1, s * 0.2, s * 0.4);

    // Canopy (layered circles for depth)
    ctx.fillStyle = '#1a4a1a';
    ctx.beginPath();
    ctx.arc(px, py - s * 0.2, s * 0.45, 0, Math.PI * 2);
    ctx.fill();
    ctx.fillStyle = '#2a5a2a';
    ctx.beginPath();
    ctx.arc(px - s * 0.1, py - s * 0.3, s * 0.35, 0, Math.PI * 2);
    ctx.fill();
    ctx.fillStyle = '#3a6a3a';
    ctx.beginPath();
    ctx.arc(px + s * 0.05, py - s * 0.35, s * 0.25, 0, Math.PI * 2);
    ctx.fill();
}

function drawMine(a) {
    const px = offsetX + a.x * scale + scale / 2;
    const py = offsetY + a.y * scale + scale / 2;
    const s = Math.max(2, scale * 0.35);
    ctx.fillStyle = '#8a6630';
    ctx.beginPath();
    ctx.arc(px, py, s, 0, Math.PI * 2);
    ctx.fill();
    ctx.strokeStyle = '#6a4a20';
    ctx.lineWidth = 1;
    ctx.stroke();
}

function drawBuilding(a) {
    const fp = BUILDING_FOOTPRINTS[a.actor_type] || [2, 2];
    const bx = offsetX + a.x * scale;
    const by = offsetY + a.y * scale;
    const bw = fp[0] * scale;
    const bh = fp[1] * scale;
    const selected = selectedUnits.includes(a.id);
    const color = getColor(a.owner);
    const light = getLightColor(a.owner);
    const dark = getDarkColor(a.owner);

    // Try to draw sprite
    const sprite = spriteImages[a.actor_type];
    if (sprite && sprite.length > 0) {
        const frameIdx = 0;
        const bmp = sprite[frameIdx];
        if (bmp) {
            // Draw sprite scaled to building footprint
            ctx.drawImage(bmp, bx, by - bh * 0.1, bw, bh * 1.1);
            if (selected) {
                ctx.strokeStyle = '#44ff44';
                ctx.lineWidth = 2;
                ctx.strokeRect(bx - 1, by - 1, bw + 2, bh + 2);
            }
            if (a.max_hp > 0 && a.hp < a.max_hp) {
                drawHealthBar(bx, by - 5, bw, 4, a.hp / a.max_hp);
            }
            return;
        }
    }

    // Fallback: nice looking building shapes
    // Shadow
    ctx.fillStyle = 'rgba(0,0,0,0.25)';
    ctx.fillRect(bx + 3, by + 3, bw - 2, bh - 2);

    // Building base
    const grad = ctx.createLinearGradient(bx, by, bx, by + bh);
    grad.addColorStop(0, light);
    grad.addColorStop(1, dark);
    ctx.fillStyle = grad;
    ctx.fillRect(bx + 1, by + 1, bw - 2, bh - 2);

    // Top edge highlight
    ctx.fillStyle = light;
    ctx.fillRect(bx + 1, by + 1, bw - 2, Math.max(2, bh * 0.15));

    // Building outline
    ctx.strokeStyle = 'rgba(0,0,0,0.6)';
    ctx.lineWidth = 1;
    ctx.strokeRect(bx + 1, by + 1, bw - 2, bh - 2);

    // Inner detail lines (windows/panels)
    if (scale > 4) {
        ctx.strokeStyle = 'rgba(0,0,0,0.15)';
        ctx.lineWidth = 0.5;
        const cols = fp[0];
        for (let i = 1; i < cols; i++) {
            const lx = bx + i * scale;
            ctx.beginPath(); ctx.moveTo(lx, by + 2); ctx.lineTo(lx, by + bh - 2); ctx.stroke();
        }
        for (let i = 1; i < fp[1]; i++) {
            const ly = by + i * scale;
            ctx.beginPath(); ctx.moveTo(bx + 2, ly); ctx.lineTo(bx + bw - 2, ly); ctx.stroke();
        }
    }

    // Building type label
    if (scale > 3 && a.actor_type) {
        ctx.fillStyle = '#fff';
        ctx.font = `bold ${Math.max(7, scale * 0.65)}px monospace`;
        ctx.textAlign = 'center';
        ctx.shadowColor = '#000';
        ctx.shadowBlur = 3;
        ctx.fillText(a.actor_type, bx + bw / 2, by + bh / 2 + scale * 0.2);
        ctx.shadowBlur = 0;
    }

    // Selection highlight
    if (selected) {
        ctx.strokeStyle = '#44ff44';
        ctx.lineWidth = 2;
        ctx.strokeRect(bx - 1, by - 1, bw + 2, bh + 2);
    }

    // Health bar
    if (a.max_hp > 0 && a.hp < a.max_hp) {
        drawHealthBar(bx, by - 5, bw, 4, a.hp / a.max_hp);
    }
}

function drawUnit(a) {
    const px = offsetX + a.x * scale + scale / 2;
    const py = offsetY + a.y * scale + scale / 2;
    const color = getColor(a.owner);
    const light = getLightColor(a.owner);
    const dark = getDarkColor(a.owner);
    const selected = selectedUnits.includes(a.id);

    // Try sprite
    const sprite = spriteImages[a.actor_type];
    if (sprite && sprite.length > 0) {
        const info = spriteInfo[a.actor_type];
        if (info) {
            const sw = info.width * scale / 24;
            const sh = info.height * scale / 24;
            ctx.drawImage(sprite[0], px - sw / 2, py - sh / 2, sw, sh);
            if (selected) {
                ctx.strokeStyle = '#44ff44'; ctx.lineWidth = 2;
                ctx.beginPath(); ctx.arc(px, py, Math.max(sw, sh) / 2 + 2, 0, Math.PI * 2); ctx.stroke();
            }
            if (a.max_hp > 0 && a.hp < a.max_hp) {
                drawHealthBar(px - sw / 2, py - sh / 2 - 5, sw, 3, a.hp / a.max_hp);
            }
            return;
        }
    }

    // Shadow
    ctx.fillStyle = 'rgba(0,0,0,0.2)';
    ctx.beginPath();
    ctx.ellipse(px + 1, py + 2, scale * 0.3, scale * 0.15, 0, 0, Math.PI * 2);
    ctx.fill();

    if (a.kind === 'Infantry') {
        const r = Math.max(2, scale * 0.25);
        // Body
        ctx.fillStyle = dark;
        ctx.fillRect(px - r * 0.6, py - r * 0.3, r * 1.2, r * 1.2);
        // Head
        ctx.fillStyle = color;
        ctx.beginPath();
        ctx.arc(px, py - r * 0.5, r * 0.5, 0, Math.PI * 2);
        ctx.fill();
        // Gun flash when attacking
        if (a.activity === 'attacking') {
            ctx.fillStyle = '#ffff44';
            ctx.beginPath();
            ctx.arc(px + r, py - r * 0.3, r * 0.3, 0, Math.PI * 2);
            ctx.fill();
        }
    } else if (a.kind === 'Vehicle' || a.kind === 'Mcv') {
        const r = Math.max(3, scale * 0.4);
        // Tank body - rounded rectangle
        const tw = r * 1.8;
        const th = r * 1.2;
        ctx.fillStyle = dark;
        roundRect(px - tw / 2, py - th / 2, tw, th, r * 0.2);
        ctx.fill();
        // Top
        ctx.fillStyle = color;
        roundRect(px - tw * 0.35, py - th * 0.35, tw * 0.7, th * 0.7, r * 0.15);
        ctx.fill();
        // Highlight
        ctx.fillStyle = light;
        ctx.fillRect(px - tw * 0.3, py - th * 0.3, tw * 0.6, th * 0.15);
        // Turret (if tank)
        if (a.actor_type && a.actor_type.includes('tnk')) {
            ctx.fillStyle = color;
            ctx.beginPath();
            ctx.arc(px, py, r * 0.3, 0, Math.PI * 2);
            ctx.fill();
            // Gun barrel
            ctx.strokeStyle = dark;
            ctx.lineWidth = Math.max(1, r * 0.15);
            ctx.beginPath();
            ctx.moveTo(px, py);
            ctx.lineTo(px + r * 0.7, py);
            ctx.stroke();
        }
        // Harvester indicator
        if (a.actor_type === 'harv' && a.activity === 'harvesting') {
            ctx.strokeStyle = '#ffcc00'; ctx.lineWidth = 2;
            ctx.beginPath(); ctx.arc(px, py, r + 2, 0, Math.PI * 2); ctx.stroke();
        }
        // MCV label
        if (a.kind === 'Mcv' && scale > 4) {
            ctx.fillStyle = '#fff';
            ctx.font = `bold ${Math.max(6, scale * 0.35)}px monospace`;
            ctx.textAlign = 'center';
            ctx.fillText('MCV', px, py + r * 0.15);
        }
    } else if (a.kind === 'Aircraft') {
        const r = Math.max(3, scale * 0.4);
        // Aircraft body
        ctx.fillStyle = color;
        ctx.beginPath();
        ctx.moveTo(px, py - r); ctx.lineTo(px - r * 0.5, py + r * 0.3);
        ctx.lineTo(px - r * 0.15, py + r * 0.5); ctx.lineTo(px + r * 0.15, py + r * 0.5);
        ctx.lineTo(px + r * 0.5, py + r * 0.3);
        ctx.closePath();
        ctx.fill();
        // Wings
        ctx.fillStyle = dark;
        ctx.fillRect(px - r, py - r * 0.1, r * 2, r * 0.3);
    } else if (a.kind === 'Ship') {
        const r = Math.max(3, scale * 0.4);
        ctx.fillStyle = color;
        ctx.beginPath();
        ctx.moveTo(px, py - r); ctx.lineTo(px + r * 0.6, py + r * 0.3);
        ctx.lineTo(px, py + r); ctx.lineTo(px - r * 0.6, py + r * 0.3);
        ctx.closePath();
        ctx.fill();
    }

    // Selection ring
    if (selected) {
        const r = Math.max(4, scale * 0.45);
        ctx.strokeStyle = '#44ff44';
        ctx.lineWidth = 2;
        ctx.beginPath();
        ctx.ellipse(px, py + 1, r, r * 0.5, 0, 0, Math.PI * 2);
        ctx.stroke();
    }

    // Unit type label
    if (scale > 6 && a.actor_type) {
        ctx.fillStyle = '#fff';
        ctx.font = `bold ${Math.max(6, scale * 0.3)}px monospace`;
        ctx.textAlign = 'center';
        ctx.shadowColor = '#000'; ctx.shadowBlur = 2;
        ctx.fillText(a.actor_type, px, py + scale * 0.55);
        ctx.shadowBlur = 0;
    }

    // Health bar
    if (a.max_hp > 0 && a.hp < a.max_hp) {
        const bw = Math.max(8, scale * 0.8);
        drawHealthBar(px - bw / 2, py - scale * 0.45 - 5, bw, 3, a.hp / a.max_hp);
    }
}

function drawPlacementGhost() {
    const [fw, fh] = placementMode.footprint;
    const gx = offsetX + mouseCell.x * scale;
    const gy = offsetY + mouseCell.y * scale;
    const canPlace = session?.can_place_building?.(placementMode.type, mouseCell.x, mouseCell.y) ?? false;

    ctx.fillStyle = canPlace ? 'rgba(68,204,68,0.25)' : 'rgba(204,68,68,0.25)';
    ctx.fillRect(gx, gy, fw * scale, fh * scale);
    ctx.strokeStyle = canPlace ? '#44cc44' : '#cc4444';
    ctx.lineWidth = 2;
    ctx.setLineDash([4, 4]);
    ctx.strokeRect(gx, gy, fw * scale, fh * scale);
    ctx.setLineDash([]);
    if (scale > 3) {
        ctx.fillStyle = '#fff';
        ctx.font = `${Math.max(8, scale * 0.5)}px monospace`;
        ctx.textAlign = 'center';
        ctx.shadowColor = '#000'; ctx.shadowBlur = 2;
        ctx.fillText(placementMode.type, gx + fw * scale / 2, gy + fh * scale / 2 + scale * 0.2);
        ctx.shadowBlur = 0;
    }
}

function drawReplayOverlay(snapshot) {
    ctx.textAlign = 'left';
    let py = 16;
    ctx.font = 'bold 13px monospace';
    for (const p of snapshot.players) {
        ctx.fillStyle = getColor(p.index);
        ctx.shadowColor = '#000'; ctx.shadowBlur = 3;
        let powerStr = '';
        if (p.power_provided > 0 || p.power_drained > 0) {
            const low = p.power_drained > p.power_provided;
            powerStr = ` | Power: ${p.power_provided}/${p.power_drained}${low ? ' LOW' : ''}`;
        }
        ctx.fillText(`P${p.index}: $${p.cash}${powerStr}`, 8, py);
        py += 18;
    }
    ctx.shadowBlur = 0;
}

function drawHealthBar(x, y, w, h, ratio) {
    // Background
    ctx.fillStyle = 'rgba(0,0,0,0.7)';
    ctx.fillRect(x - 1, y - 1, w + 2, h + 2);
    // Health
    ctx.fillStyle = ratio > 0.5 ? '#44cc44' : ratio > 0.25 ? '#cccc44' : '#cc4444';
    ctx.fillRect(x, y, w * ratio, h);
}

function roundRect(x, y, w, h, r) {
    ctx.beginPath();
    ctx.moveTo(x + r, y);
    ctx.lineTo(x + w - r, y);
    ctx.quadraticCurveTo(x + w, y, x + w, y + r);
    ctx.lineTo(x + w, y + h - r);
    ctx.quadraticCurveTo(x + w, y + h, x + w - r, y + h);
    ctx.lineTo(x + r, y + h);
    ctx.quadraticCurveTo(x, y + h, x, y + h - r);
    ctx.lineTo(x, y + r);
    ctx.quadraticCurveTo(x, y, x + r, y);
    ctx.closePath();
}

// ── Init ──
await init();
await loadSprites();
showScreen('home');
