import init, { ReplayViewer, GameSession, SpriteAtlas } from './pkg/openra_wasm.js';

// ── DOM refs ──
const homeEl = document.getElementById('home');
const replaySetupEl = document.getElementById('replay-setup');
const gameUiEl = document.getElementById('game-ui');
const canvas = document.getElementById('canvas');
const ctx = canvas.getContext('2d');
ctx.imageSmoothingEnabled = false;
const minimapCanvas = document.getElementById('minimap-canvas');
const mmCtx = minimapCanvas.getContext('2d');

const hudCash = document.getElementById('hud-cash');
const hudPower = document.getElementById('hud-power');
const hudTick = document.getElementById('hud-tick');
const hudMsg = document.getElementById('hud-msg');
const gameInfo = document.getElementById('game-info');

const selSection = document.getElementById('sel-section');
const selInfo = document.getElementById('sel-info');
const selActions = document.getElementById('sel-actions');
const queueSection = document.getElementById('queue-section');
const queueList = document.getElementById('queue-list');
const prodPanel = document.getElementById('prod-panel');
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
let mouseCell = { x: -1, y: -1 };
let atlas = null;
let spriteImages = {};
let spriteInfo = {};
let paletteRGB = null;
let tilesetTemplates = {};
let mapTiles = null;
let playerSpriteCache = {};
let activeTab = 'buildings';
let currentTick = 0;

// Camera
let mapW = 128, mapH = 128;
let cellPx = 24;
let camX = 0, camY = 0;
const CELL_PX = 24;

// ── Player colors (OpenRA defaults) ──
const PLAYER_COLORS = {
    0: '#888', 1: '#888', 2: '#888',
    3: '#4488dd', 4: '#dd4444', 5: '#44dd44',
    6: '#dddd44', 7: '#dd44dd',
};
const PLAYER_COLORS_RGB = {
    0: [136,136,136], 1: [136,136,136], 2: [136,136,136],
    3: [68,136,221], 4: [221,68,68], 5: [68,221,68],
    6: [221,221,68], 7: [221,68,221],
};
const REMAP_START = 80, REMAP_END = 95;

// Building footprints (cells)
const BUILDING_FOOTPRINTS = {
    'fact': [3,2], 'weap': [3,2], 'weap.ukraine': [3,2], 'proc': [3,2],
    'fix': [3,2], 'spen': [3,3], 'syrd': [3,3],
    'powr': [2,2], 'apwr': [2,2], 'tent': [2,2], 'barr': [2,2],
    'dome': [2,2], 'hpad': [2,2], 'afld': [2,2], 'atek': [2,2],
    'stek': [2,2], 'iron': [2,2], 'gap': [1,2],
    'tsla': [1,2], 'sam': [2,1],
    'pbox': [1,1], 'hbox': [1,1], 'gun': [1,1], 'ftur': [1,1],
    'silo': [1,1], 'agun': [1,1],
    // Newly added
    'miss': [2,2], 'pdox': [2,2], 'fcom': [2,2],
    'hosp': [2,2], 'bio': [2,2], 'oilb': [2,2], 'kenn': [1,1],
    'brik': [1,1], 'sbag': [1,1], 'fenc': [1,1], 'cycl': [1,1],
    'barb': [1,1],
};

// Building overlay sprites (rendered on top of base sprite)
const BUILDING_OVERLAYS = {
    'proc': ['proctop'],
    'sam': ['sam2'],
};

// Building foundation bibs (rendered under building sprite)
const BUILDING_BIBS = {
    'gun': 'ter:mbGUN.tem', 'agun': 'ter:mbAGUN.tem',
    'ftur': 'ter:mbFTUR.tem', 'pbox': 'ter:mbPBOX.tem',
    'sam': 'ter:mbSAM.tem', 'tsla': 'ter:mbTSLA.tem',
    'silo': 'ter:mbSILO.tem', 'gap': 'ter:mbGAP.tem',
    'iron': 'ter:mbIRON.tem', 'fix': 'ter:mbFIX.tem',
    'pdox': 'ter:mbPDOX.tem', 'hosp': 'ter:mbHOSP.tem',
};

// Helicopter rotor overlays (animated)
const ROTOR_SPRITES = {
    'heli': 'lrotorlg', 'hind': 'lrotorlg',
    'tran': 'lrotor', 'mh60': 'yrotorlg',
};

// Destroyed unit husk sprites
const HUSK_SPRITES = {
    'heli': 'hhusk', 'hind': 'hhusk2',
    'tran': 'tran1husk', 'tran2': 'tran2husk',
    'mcv': 'mcvhusk',
};

// Ship turret overlays (32 facings)
const SHIP_TURRETS = {
    'ca': 'turr', 'dd': 'ssam', 'pt': 'mgun',
};

// ── HSV color remapping ──
function rgb2hsv(r, g, b) {
    r /= 255; g /= 255; b /= 255;
    const mx = Math.max(r, g, b), mn = Math.min(r, g, b), d = mx - mn;
    let h = 0, s = mx === 0 ? 0 : d / mx;
    if (d !== 0) {
        if (mx === r) h = ((g - b) / d + 6) % 6;
        else if (mx === g) h = (b - r) / d + 2;
        else h = (r - g) / d + 4;
        h /= 6;
    }
    return [h, s, mx];
}
function hsv2rgb(h, s, v) {
    const i = Math.floor(h * 6), f = h * 6 - i;
    const p = v*(1-s), q = v*(1-f*s), t = v*(1-(1-f)*s);
    let r, g, b;
    switch (i % 6) {
        case 0: r=v;g=t;b=p; break; case 1: r=q;g=v;b=p; break;
        case 2: r=p;g=v;b=t; break; case 3: r=p;g=q;b=v; break;
        case 4: r=t;g=p;b=v; break; case 5: r=v;g=p;b=q; break;
    }
    return [Math.round(r*255), Math.round(g*255), Math.round(b*255)];
}

async function getPlayerSprite(name, frameIdx, ownerColor) {
    if (!ownerColor) return spriteImages[name]?.[frameIdx] || null;
    const key = `${name}:${frameIdx}:${ownerColor.join(',')}`;
    if (playerSpriteCache[key]) return playerSpriteCache[key];
    const info = spriteInfo[name];
    if (!info) return null;
    const indexed = atlas.frame_indexed(name, frameIdx);
    if (!indexed || indexed.length === 0) return spriteImages[name]?.[frameIdx] || null;
    const [pH, pS] = rgb2hsv(...ownerColor);
    const w = info.width, h = info.height;
    const rgba = new Uint8ClampedArray(w * h * 4);
    for (let i = 0; i < indexed.length; i++) {
        const palIdx = indexed[i];
        if (palIdx === 0) { rgba[i*4+3] = 0; }
        else if (palIdx === 4) { rgba[i*4]=0; rgba[i*4+1]=0; rgba[i*4+2]=0; rgba[i*4+3]=128; }
        else if (palIdx >= REMAP_START && palIdx <= REMAP_END) {
            const [,,origV] = rgb2hsv(paletteRGB[palIdx*3], paletteRGB[palIdx*3+1], paletteRGB[palIdx*3+2]);
            const [r,g,b] = hsv2rgb(pH, pS, origV);
            rgba[i*4]=r; rgba[i*4+1]=g; rgba[i*4+2]=b; rgba[i*4+3]=255;
        } else {
            rgba[i*4]=paletteRGB[palIdx*3]; rgba[i*4+1]=paletteRGB[palIdx*3+1];
            rgba[i*4+2]=paletteRGB[palIdx*3+2]; rgba[i*4+3]=255;
        }
    }
    const bmp = await createImageBitmap(new ImageData(rgba, w, h));
    playerSpriteCache[key] = bmp;
    return bmp;
}

async function pregenPlayerSprites(playerIdx) {
    const color = PLAYER_COLORS_RGB[playerIdx];
    if (!color) return;
    for (const [name, info] of Object.entries(spriteInfo)) {
        if (name.startsWith('ter:')) continue;
        const max = Math.min(info.frames, 32);
        for (let f = 0; f < max; f++) await getPlayerSprite(name, f, color);
    }
}

// ── Sprite loading ──
async function loadSprites() {
    try {
        atlas = new SpriteAtlas();
        spriteInfo = JSON.parse(atlas.info_json());
        paletteRGB = atlas.palette_rgb();
        tilesetTemplates = JSON.parse(atlas.tileset_json());
        for (const [name, info] of Object.entries(spriteInfo)) {
            spriteImages[name] = [];
            for (let i = 0; i < info.frames; i++) {
                const rgba = atlas.frame_rgba(name, i);
                if (rgba.length > 0) {
                    const bmp = await createImageBitmap(new ImageData(new Uint8ClampedArray(rgba), info.width, info.height));
                    spriteImages[name].push(bmp);
                } else {
                    spriteImages[name].push(null);
                }
            }
        }
        console.log('Loaded', Object.keys(spriteInfo).length, 'sprites');
    } catch (e) { console.error('Sprite loading failed:', e); }
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

// ── Replay ──
let replayBytes = null, mapBytes = null;
document.getElementById('replay-file').addEventListener('change', async e => {
    const f = e.target.files[0];
    if (f) replayBytes = new Uint8Array(await f.arrayBuffer());
    document.getElementById('btn-load-replay').disabled = !(replayBytes && mapBytes);
});
document.getElementById('map-file').addEventListener('change', async e => {
    const f = e.target.files[0];
    if (f) mapBytes = new Uint8Array(await f.arrayBuffer());
    document.getElementById('btn-load-replay').disabled = !(replayBytes && mapBytes);
});
document.getElementById('btn-load-replay').addEventListener('click', () => {
    try {
        session = new ReplayViewer(replayBytes, mapBytes);
        mode = 'replay'; humanPlayerId = null; selectedUnits = []; placementMode = null; playing = false;
        mapTiles = JSON.parse(session.map_tiles_json());
        replayControlsEl.style.display = 'block';
        document.getElementById('cmd-deploy').parentElement.style.display = 'none';
        showScreen('game'); resizeCanvas();
        lastSnapshot = JSON.parse(session.snapshot_json());
        buildTerrainCanvas(lastSnapshot);
        centerCamera(lastSnapshot); render(lastSnapshot);
    } catch (e) { document.getElementById('replay-status').textContent = `Error: ${e}`; }
});
document.getElementById('btn-play').onclick = () => {
    if (playing) { playing = false; document.getElementById('btn-play').textContent = 'Play'; }
    else { playing = true; document.getElementById('btn-play').textContent = 'Pause'; replayLoop(); }
};
document.getElementById('btn-step').onclick = () => { if (session) replayStep(); };
document.getElementById('speed').oninput = e => { document.getElementById('speed-val').textContent = e.target.value; };

function replayStep() {
    if (!session.tick()) { playing = false; showMsg('Replay finished'); return false; }
    currentTick++;
    lastSnapshot = JSON.parse(session.snapshot_json());
    render(lastSnapshot); updateHUD(lastSnapshot);
    hudTick.textContent = `${session.current_frame()}/${session.total_frames()}`;
    return true;
}
function replayLoop() {
    if (!playing || !session) return;
    const fps = parseInt(document.getElementById('speed').value);
    for (let i = 0; i < fps; i++) { if (!replayStep()) return; }
    requestAnimationFrame(replayLoop);
}

// ── Game start ──
async function startGame() {
    try {
        session = new GameSession();
        mode = 'game'; humanPlayerId = session.human_player_id();
        selectedUnits = []; placementMode = null; playing = true;
        PLAYER_COLORS[humanPlayerId] = '#4488dd';
        PLAYER_COLORS[humanPlayerId+1] = '#dd4444';
        PLAYER_COLORS_RGB[humanPlayerId] = [68,136,221];
        PLAYER_COLORS_RGB[humanPlayerId+1] = [221,68,68];
        mapTiles = JSON.parse(session.map_tiles_json());
        await pregenPlayerSprites(humanPlayerId);
        await pregenPlayerSprites(humanPlayerId+1);
        replayControlsEl.style.display = 'none';
        document.getElementById('cmd-deploy').parentElement.style.display = 'flex';
        showScreen('game'); resizeCanvas();
        lastSnapshot = JSON.parse(session.snapshot_json());
        buildTerrainCanvas(lastSnapshot);
        centerCamera(lastSnapshot);
        refreshBuildable();
        gameLoop();
    } catch (e) { alert(`Failed: ${e}`); }
}

function centerCamera(snapshot) {
    mapW = snapshot.map_width || 128;
    mapH = snapshot.map_height || 128;
    const myActors = snapshot.actors.filter(a => a.owner === (humanPlayerId || 3));
    if (myActors.length > 0) {
        const avgX = myActors.reduce((s,a) => s + a.x, 0) / myActors.length;
        const avgY = myActors.reduce((s,a) => s + a.y, 0) / myActors.length;
        camX = avgX * cellPx - canvas.width / 2;
        camY = avgY * cellPx - canvas.height / 2;
    }
}

function gameLoop() {
    if (!playing || mode !== 'game') return;
    session.tick();
    currentTick++;
    lastSnapshot = JSON.parse(session.snapshot_json());
    render(lastSnapshot); updateHUD(lastSnapshot);
    if (session.current_frame() % 30 === 0) refreshBuildable();
    const winner = session.winner();
    if (winner > 0) {
        playing = false;
        showMsg(winner === humanPlayerId ? 'Victory!' : 'Defeated');
        return;
    }
    setTimeout(gameLoop, 40);
}

function showMsg(text) { hudMsg.textContent = text; setTimeout(() => { if (hudMsg.textContent === text) hudMsg.textContent = ''; }, 3000); }

// ── Production tab switching ──
document.querySelectorAll('.prod-tab').forEach(tab => {
    tab.addEventListener('click', () => {
        activeTab = tab.dataset.tab;
        document.querySelectorAll('.prod-tab').forEach(t => t.classList.remove('active'));
        tab.classList.add('active');
        renderBuildable();
    });
});

function refreshBuildable() {
    if (mode !== 'game') return;
    try { buildableItems = JSON.parse(session.buildable_items_json()); } catch { buildableItems = []; }
    renderBuildable();
}

function renderBuildable() {
    if (!buildableItems) return;
    const items = activeTab === 'buildings'
        ? buildableItems.filter(i => i.is_building)
        : buildableItems.filter(i => !i.is_building);
    prodPanel.innerHTML = '';
    for (const item of items) {
        const btn = document.createElement('button');
        btn.className = 'prod-icon';
        // Try to render sprite icon
        const iconInfo = spriteInfo[item.name];
        const iconFrames = spriteImages[item.name];
        if (iconInfo && iconFrames && iconFrames[0]) {
            const iconCanvas = document.createElement('canvas');
            iconCanvas.width = 62; iconCanvas.height = 36;
            const ictx = iconCanvas.getContext('2d');
            const aspect = iconInfo.width / iconInfo.height;
            let iw = 62, ih = 36;
            if (aspect > 62/36) { ih = Math.round(62 / aspect); } else { iw = Math.round(36 * aspect); }
            ictx.drawImage(iconFrames[0], (62-iw)/2, (36-ih)/2, iw, ih);
            iconCanvas.style.cssText = 'display:block;width:100%;image-rendering:pixelated;';
            btn.appendChild(iconCanvas);
        }
        const nameSpan = document.createElement('span');
        nameSpan.className = 'name'; nameSpan.textContent = item.name;
        const costSpan = document.createElement('span');
        costSpan.className = 'cost'; costSpan.textContent = `$${item.cost}`;
        btn.appendChild(nameSpan);
        btn.appendChild(costSpan);
        btn.onclick = () => session.order_start_production(item.name);
        prodPanel.appendChild(btn);
    }
    if (lastSnapshot) refreshQueue(lastSnapshot);
}

function refreshQueue(snapshot) {
    if (mode !== 'game' || !snapshot) return;
    const myPlayer = snapshot.players.find(p => p.index === humanPlayerId);
    if (!myPlayer?.production_queue?.length) { queueSection.style.display = 'none'; return; }
    queueSection.style.display = 'block';
    queueList.innerHTML = '';
    for (const item of myPlayer.production_queue) {
        const pct = Math.round(item.progress * 100);
        const div = document.createElement('div');
        div.style.cssText = 'font-size:10px;padding:1px 0;';
        div.textContent = `${item.item_name} ${pct}%`;
        if (item.done) {
            div.style.color = '#4a8a2a';
            const bi = buildableItems?.find(b => b.name === item.item_name && b.is_building);
            if (bi) {
                div.style.cursor = 'pointer';
                div.textContent += ' [PLACE]';
                div.onclick = () => {
                    placementMode = { type: item.item_name, footprint: [bi.footprint[0], bi.footprint[1]] };
                    showMsg(`Place ${item.item_name}`);
                };
                if (!placementMode) {
                    placementMode = { type: item.item_name, footprint: [bi.footprint[0], bi.footprint[1]] };
                }
            }
        }
        queueList.appendChild(div);
    }
    // Update production icons with progress bars
    const icons = prodPanel.querySelectorAll('.prod-icon');
    icons.forEach(icon => {
        const name = icon.querySelector('.name')?.textContent;
        const qi = myPlayer.production_queue.find(q => q.item_name === name);
        let bar = icon.querySelector('.progress-bar');
        if (qi) {
            if (!bar) { bar = document.createElement('div'); bar.className = 'progress-bar'; icon.appendChild(bar); }
            bar.style.width = `${qi.progress * 100}%`;
            if (qi.done) { bar.classList.add('done'); icon.classList.add('building-ready'); }
            else { bar.classList.remove('done'); icon.classList.remove('building-ready'); }
        } else if (bar) { bar.remove(); icon.classList.remove('building-ready'); }
    });
}

function refreshSelection() {
    if (selectedUnits.length === 0 || !lastSnapshot) { selSection.style.display = 'none'; return; }
    selSection.style.display = 'block';
    const actors = lastSnapshot.actors.filter(a => selectedUnits.includes(a.id));
    if (actors.length === 0) { selSection.style.display = 'none'; return; }
    if (actors.length === 1) {
        const a = actors[0];
        const hpPct = a.max_hp > 0 ? Math.round(a.hp / a.max_hp * 100) : 100;
        selInfo.innerHTML = `<span class="name">${a.actor_type || a.kind}</span> HP:${hpPct}% ${a.activity}`;
    } else {
        selInfo.innerHTML = `<span class="name">${actors.length} units</span>`;
    }
    selActions.innerHTML = '';
}

function updateHUD(snapshot) {
    if (!snapshot) return;
    const myPlayer = snapshot.players.find(p => p.index === humanPlayerId);
    if (myPlayer) {
        hudCash.textContent = `$${myPlayer.cash}`;
        const low = myPlayer.power_drained > myPlayer.power_provided;
        hudPower.textContent = `${myPlayer.power_provided}/${myPlayer.power_drained}`;
        hudPower.className = 'power' + (low ? ' low' : '');
    } else if (mode === 'replay' && snapshot.players.length > 0) {
        let info = '';
        for (const p of snapshot.players) info += `P${p.index}:$${p.cash} `;
        gameInfo.textContent = info;
        hudCash.textContent = `$${snapshot.players[0]?.cash || 0}`;
    }
}

// ── Canvas resize ──
function resizeCanvas() {
    const wrap = document.getElementById('canvas-wrap');
    canvas.width = wrap.clientWidth;
    canvas.height = wrap.clientHeight;
    ctx.imageSmoothingEnabled = false;
}
window.addEventListener('resize', () => { resizeCanvas(); if (lastSnapshot) render(lastSnapshot); });

// ── Coordinate conversion ──
function screenToWorld(sx, sy) {
    return { x: Math.floor((sx + camX) / cellPx), y: Math.floor((sy + camY) / cellPx) };
}
function actorAtCell(cx, cy, snapshot) {
    for (const a of snapshot.actors) {
        if (a.kind === 'Building') {
            const fp = BUILDING_FOOTPRINTS[a.actor_type] || [2,2];
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
canvas.addEventListener('mousemove', e => {
    const rect = canvas.getBoundingClientRect();
    mouseCell = screenToWorld(e.clientX - rect.left, e.clientY - rect.top);
    if (placementMode && lastSnapshot) render(lastSnapshot);
});

canvas.addEventListener('click', e => {
    if (!lastSnapshot) return;
    const rect = canvas.getBoundingClientRect();
    const cell = screenToWorld(e.clientX - rect.left, e.clientY - rect.top);
    if (mode === 'game') handleGameClick(cell, e.shiftKey);
    else if (mode === 'replay') {
        const actor = actorAtCell(cell.x, cell.y, lastSnapshot);
        selectedUnits = actor ? [actor.id] : [];
        refreshSelection();
    }
});

canvas.addEventListener('contextmenu', e => {
    e.preventDefault();
    if (mode !== 'game' || !lastSnapshot) return;
    const rect = canvas.getBoundingClientRect();
    const cell = screenToWorld(e.clientX - rect.left, e.clientY - rect.top);
    if (placementMode) { placementMode = null; showMsg(''); return; }
    if (selectedUnits.length === 0) return;
    const target = actorAtCell(cell.x, cell.y, lastSnapshot);
    if (target && target.owner !== humanPlayerId && target.owner > 2) {
        for (const uid of selectedUnits) session.order_attack(uid, target.id);
    } else {
        for (const uid of selectedUnits) session.order_move(uid, cell.x, cell.y);
    }
});

function handleGameClick(cell, shiftKey) {
    if (placementMode) {
        if (session.can_place_building(placementMode.type, cell.x, cell.y)) {
            session.order_place_building(placementMode.type, cell.x, cell.y);
            placementMode = null; refreshBuildable();
        } else { showMsg('Cannot place here'); }
        return;
    }
    const actor = actorAtCell(cell.x, cell.y, lastSnapshot);
    if (actor && actor.owner === humanPlayerId) {
        if (shiftKey) { if (!selectedUnits.includes(actor.id)) selectedUnits.push(actor.id); }
        else selectedUnits = [actor.id];
    } else if (actor && actor.owner !== humanPlayerId && actor.owner > 2 && selectedUnits.length > 0) {
        for (const uid of selectedUnits) session.order_attack(uid, actor.id);
    } else { if (!shiftKey) selectedUnits = []; }
    refreshSelection();
}

// Drag select
let dragStart = null;
canvas.addEventListener('mousedown', e => {
    if (e.button !== 0) return;
    const rect = canvas.getBoundingClientRect();
    dragStart = { x: e.clientX - rect.left, y: e.clientY - rect.top };
});
canvas.addEventListener('mouseup', e => {
    if (e.button !== 0 || !dragStart || !lastSnapshot || mode !== 'game') { dragStart = null; return; }
    const rect = canvas.getBoundingClientRect();
    const end = { x: e.clientX - rect.left, y: e.clientY - rect.top };
    if (Math.abs(end.x - dragStart.x) > 10 || Math.abs(end.y - dragStart.y) > 10) {
        const c1 = screenToWorld(Math.min(dragStart.x, end.x), Math.min(dragStart.y, end.y));
        const c2 = screenToWorld(Math.max(dragStart.x, end.x), Math.max(dragStart.y, end.y));
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

// ── Keyboard ──
document.addEventListener('keydown', e => {
    if (mode !== 'game') return;
    if (e.key === 'Escape') {
        if (placementMode) { placementMode = null; showMsg(''); }
        else { selectedUnits = []; refreshSelection(); }
    }
    if (e.key === 's' || e.key === 'S') for (const uid of selectedUnits) session.order_stop(uid);
    if (e.key === 'd' || e.key === 'D') {
        if (!lastSnapshot) return;
        for (const uid of selectedUnits) {
            const a = lastSnapshot.actors.find(a => a.id === uid);
            if (a && a.kind === 'Mcv') session.order_deploy(uid);
        }
    }
    const sp = 20;
    if (e.key === 'ArrowLeft') camX -= sp;
    if (e.key === 'ArrowRight') camX += sp;
    if (e.key === 'ArrowUp') camY -= sp;
    if (e.key === 'ArrowDown') camY += sp;
    if (e.key === '+' || e.key === '=') cellPx = Math.min(96, cellPx + 4);
    if (e.key === '-') cellPx = Math.max(8, cellPx - 4);
});

// Command bar buttons
document.getElementById('cmd-stop')?.addEventListener('click', () => {
    for (const uid of selectedUnits) session.order_stop(uid);
});
document.getElementById('cmd-deploy')?.addEventListener('click', () => {
    if (!lastSnapshot) return;
    for (const uid of selectedUnits) {
        const a = lastSnapshot.actors.find(a => a.id === uid);
        if (a && a.kind === 'Mcv') session.order_deploy(uid);
    }
});
document.getElementById('cmd-sell')?.addEventListener('click', () => {
    for (const uid of selectedUnits) session.order_sell(uid);
});

// Mouse wheel zoom
canvas.addEventListener('wheel', e => {
    e.preventDefault();
    const old = cellPx;
    cellPx = e.deltaY < 0 ? Math.min(96, cellPx + 2) : Math.max(8, cellPx - 2);
    const rect = canvas.getBoundingClientRect();
    const mx = e.clientX - rect.left, my = e.clientY - rect.top;
    camX = (camX + mx) * cellPx / old - mx;
    camY = (camY + my) * cellPx / old - my;
    if (lastSnapshot) render(lastSnapshot);
}, { passive: false });

// Edge scrolling
let edgeScrollInterval = null;
canvas.addEventListener('mousemove', e => {
    const rect = canvas.getBoundingClientRect();
    const mx = e.clientX - rect.left, my = e.clientY - rect.top;
    const margin = 16;
    let dx = 0, dy = 0;
    if (mx < margin) dx = -6; else if (mx > canvas.width - margin) dx = 6;
    if (my < margin) dy = -6; else if (my > canvas.height - margin) dy = 6;
    if (dx || dy) {
        if (!edgeScrollInterval) {
            edgeScrollInterval = setInterval(() => {
                camX += dx; camY += dy;
                if (lastSnapshot) render(lastSnapshot);
            }, 30);
        }
    } else if (edgeScrollInterval) { clearInterval(edgeScrollInterval); edgeScrollInterval = null; }
});

// Minimap click to move camera
minimapCanvas.addEventListener('click', e => {
    const rect = minimapCanvas.getBoundingClientRect();
    const mx = e.clientX - rect.left, my = e.clientY - rect.top;
    const mmScale = Math.min(222 / mapW, 222 / mapH);
    const offX = (222 - mapW * mmScale) / 2;
    const offY = (222 - mapH * mmScale) / 2;
    const cellX = (mx - offX) / mmScale;
    const cellY = (my - offY) / mmScale;
    camX = cellX * cellPx - canvas.width / 2;
    camY = cellY * cellPx - canvas.height / 2;
    if (lastSnapshot) render(lastSnapshot);
});

// ── Terrain ──
let terrainCanvas = null;

function buildTerrainCanvas(snapshot) {
    mapW = snapshot.map_width || 128;
    mapH = snapshot.map_height || 128;
    if (!mapTiles || mapTiles.length === 0) { terrainCanvas = null; return; }

    terrainCanvas = document.createElement('canvas');
    terrainCanvas.width = mapW * CELL_PX;
    terrainCanvas.height = mapH * CELL_PX;
    const tctx = terrainCanvas.getContext('2d');
    tctx.imageSmoothingEnabled = false;
    // Fill with palette color 0 (near-black, like map edges)
    tctx.fillStyle = '#0a0a0a';
    tctx.fillRect(0, 0, terrainCanvas.width, terrainCanvas.height);

    for (let row = 0; row < Math.min(mapH, mapTiles.length); row++) {
        const tileRow = mapTiles[row];
        if (!tileRow) continue;
        for (let col = 0; col < Math.min(mapW, tileRow.length); col++) {
            const [typeId, tileIndex] = tileRow[col];
            const tmpl = tilesetTemplates[typeId.toString()];
            if (!tmpl) {
                const clear = spriteImages['ter:clear1.tem'];
                if (clear) {
                    const fi = ((col * 7 + row * 13) % clear.length);
                    if (clear[fi]) tctx.drawImage(clear[fi], col * CELL_PX, row * CELL_PX);
                }
                continue;
            }
            const frames = spriteImages[`ter:${tmpl.image}`];
            if (!frames) continue;
            if (tileIndex < frames.length && frames[tileIndex]) {
                tctx.drawImage(frames[tileIndex], col * CELL_PX, row * CELL_PX);
            } else if (frames[0]) {
                tctx.drawImage(frames[0], col * CELL_PX, row * CELL_PX);
            }
        }
    }
}

// ── RENDER ──
function render(snapshot) {
    if (!snapshot) return;
    mapW = snapshot.map_width || 128;
    mapH = snapshot.map_height || 128;

    ctx.fillStyle = '#000';
    ctx.fillRect(0, 0, canvas.width, canvas.height);

    // Terrain: camX/camY are in screen pixels at current zoom (cellPx).
    // Terrain canvas is at native CELL_PX resolution.
    if (terrainCanvas) {
        // Convert camera position from screen-space to native terrain pixels
        const ratio = CELL_PX / cellPx;
        const srcX = Math.max(0, camX * ratio);
        const srcY = Math.max(0, camY * ratio);
        const srcW = canvas.width * ratio;
        const srcH = canvas.height * ratio;
        const dstX = Math.max(0, -camX);
        const dstY = Math.max(0, -camY);
        const dstW = srcW / ratio;
        const dstH = srcH / ratio;
        ctx.drawImage(terrainCanvas, srcX, srcY, srcW, srcH, dstX, dstY, dstW, dstH);
    }

    // Resources
    drawResources(snapshot);

    // Sort actors: trees/mines, buildings, units, aircraft (by y)
    const sorted = [...snapshot.actors].sort((a, b) => {
        const order = { 'Tree': 0, 'Mine': 0, 'Building': 1, 'Infantry': 2, 'Vehicle': 2, 'Mcv': 2, 'Ship': 2, 'Aircraft': 3 };
        const ka = order[a.kind] ?? 2, kb = order[b.kind] ?? 2;
        return ka !== kb ? ka - kb : (a.y - b.y || a.x - b.x);
    });

    for (const a of sorted) {
        if (a.kind === 'Tree') drawTree(a);
        else if (a.kind === 'Mine') drawMine(a);
        else if (a.kind === 'Building') drawBuilding(a);
        else drawUnit(a);
    }

    // Selection brackets + health bars on selected
    for (const a of sorted) {
        if (selectedUnits.includes(a.id)) drawSelectionBrackets(a);
    }

    // Placement ghost
    if (placementMode && mouseCell.x >= 0) drawPlacementGhost();

    // Minimap
    drawMinimap(snapshot);
}

function drawResources(snapshot) {
    if (!snapshot.resources) return;
    const scale = cellPx / CELL_PX;
    for (const res of snapshot.resources) {
        const sx = res.x * cellPx - camX, sy = res.y * cellPx - camY;
        if (sx + cellPx < 0 || sx > canvas.width || sy + cellPx < 0 || sy > canvas.height) continue;
        const d = res.density || 1;
        // Try sprite: ore = mine.tem, gems = gmine.tem
        const temName = res.kind === 1 ? 'ter:mine.tem' : 'ter:gmine.tem';
        const frames = spriteImages[temName];
        const info = spriteInfo[temName];
        if (frames && info && frames.length > 0) {
            // Pick frame based on density (more ore = later frame)
            const frame = Math.min(d - 1, frames.length - 1);
            if (frames[frame]) {
                const drawW = info.width * scale;
                const drawH = info.height * scale;
                ctx.drawImage(frames[frame], sx + cellPx/2 - drawW/2, sy + cellPx/2 - drawH/2, drawW, drawH);
                continue;
            }
        }
        // Geometric fallback
        if (res.kind === 1) {
            ctx.fillStyle = `rgba(140,110,30,${0.2 + d * 0.08})`;
            ctx.fillRect(sx + 2, sy + 2, cellPx - 4, cellPx - 4);
            ctx.fillStyle = '#a08020';
            const seed = res.x * 31 + res.y * 17;
            for (let i = 0; i < Math.min(d, 5); i++) {
                const ox = ((seed + i * 7) % (cellPx - 6)) + 3;
                const oy = ((seed + i * 13) % (cellPx - 6)) + 3;
                ctx.fillRect(sx + ox, sy + oy, 2, 2);
            }
        } else {
            ctx.fillStyle = `rgba(100,40,140,${0.3 + d * 0.08})`;
            ctx.fillRect(sx + 2, sy + 2, cellPx - 4, cellPx - 4);
            ctx.fillStyle = '#c060e0';
            const seed = res.x * 23 + res.y * 11;
            for (let i = 0; i < Math.min(d, 4); i++) {
                const ox = ((seed + i * 9) % (cellPx - 6)) + 3;
                const oy = ((seed + i * 11) % (cellPx - 6)) + 3;
                ctx.fillRect(sx + ox, sy + oy, 2, 2);
            }
        }
    }
}

function drawSprite(name, frameIdx, sx, sy, drawW, drawH, ownerIdx) {
    const color = PLAYER_COLORS_RGB[ownerIdx];
    if (color) {
        const key = `${name}:${frameIdx}:${color.join(',')}`;
        const cached = playerSpriteCache[key];
        if (cached) { ctx.drawImage(cached, sx, sy, drawW, drawH); return true; }
        getPlayerSprite(name, frameIdx, color);
    }
    const frames = spriteImages[name];
    if (!frames || !frames[frameIdx]) return false;
    ctx.drawImage(frames[frameIdx], sx, sy, drawW, drawH);
    return true;
}

function drawBuilding(a) {
    const fp = BUILDING_FOOTPRINTS[a.actor_type] || [2,2];
    const sx = a.x * cellPx - camX, sy = a.y * cellPx - camY;
    const bw = fp[0] * cellPx, bh = fp[1] * cellPx;
    if (sx + bw < 0 || sx > canvas.width || sy + bh < 0 || sy > canvas.height) return;

    const scale = cellPx / CELL_PX;
    const centerX = sx + bw / 2;
    const centerY = sy + bh / 2;

    // Render foundation bib (under building)
    const bibName = BUILDING_BIBS[a.actor_type];
    if (bibName) {
        const bibFrames = spriteImages[bibName];
        const bibInfo = spriteInfo[bibName];
        if (bibFrames && bibFrames[0] && bibInfo) {
            const bibW = bibInfo.width * scale;
            const bibH = bibInfo.height * scale;
            ctx.drawImage(bibFrames[0], centerX - bibW/2, centerY - bibH/2, bibW, bibH);
        }
    }

    const info = spriteInfo[a.actor_type];
    if (info && spriteImages[a.actor_type]) {
        let frame = 0;
        if (a.max_hp > 0 && a.hp < a.max_hp * 0.5 && info.frames > 1) frame = 1;
        const drawW = info.width * scale;
        const drawH = info.height * scale;
        const dx = centerX - drawW / 2;
        const dy = centerY - drawH / 2;
        if (drawSprite(a.actor_type, frame, dx, dy, drawW, drawH, a.owner)) {
            // Render overlays (e.g. proc ore top, SAM turret)
            const overlays = BUILDING_OVERLAYS[a.actor_type];
            if (overlays) {
                for (const ovName of overlays) {
                    const ovInfo = spriteInfo[ovName];
                    if (ovInfo && spriteImages[ovName]) {
                        const ovW = ovInfo.width * scale;
                        const ovH = ovInfo.height * scale;
                        const ovDx = centerX - ovW / 2;
                        const ovDy = centerY - ovH / 2;
                        let ovFrame = 0;
                        if (ovName === 'sam2' && ovInfo.frames >= 32 && a.facing !== undefined) {
                            const step = 1024 / 32;
                            ovFrame = Math.floor(((a.facing + step / 2) & 1023) / step) % 32;
                        }
                        drawSprite(ovName, ovFrame, ovDx, ovDy, ovW, ovH, a.owner);
                    }
                }
            }
            // Damage smoke on heavily damaged buildings
            if (a.max_hp > 0 && a.hp < a.max_hp * 0.5) {
                const smokeSprite = (a.hp < a.max_hp * 0.25) ? 'burn-l' : 'smoke_m';
                const smokeInfo = spriteInfo[smokeSprite];
                if (smokeInfo && spriteImages[smokeSprite]) {
                    const smokeFrame = (currentTick * 2) % smokeInfo.frames;
                    const sW = smokeInfo.width * scale;
                    const sH = smokeInfo.height * scale;
                    const smokeFrames = spriteImages[smokeSprite];
                    if (smokeFrames[smokeFrame]) {
                        ctx.drawImage(smokeFrames[smokeFrame], centerX - sW/2, centerY - bh/2 - sH/2, sW, sH);
                    }
                }
            }
            drawHealthBar(a, sx, sy, bw);
            return;
        }
    }
    // Fallback
    const color = PLAYER_COLORS[a.owner] || '#888';
    ctx.fillStyle = color; ctx.globalAlpha = 0.7;
    ctx.fillRect(sx, sy, bw, bh); ctx.globalAlpha = 1;
    ctx.strokeStyle = '#000'; ctx.lineWidth = 1; ctx.strokeRect(sx, sy, bw, bh);
    if (cellPx >= 10) {
        ctx.fillStyle = '#fff'; ctx.font = `bold ${Math.max(8, cellPx*0.5)}px monospace`;
        ctx.textAlign = 'center'; ctx.fillText(a.actor_type, sx + bw/2, sy + bh/2 + 4);
    }
    drawHealthBar(a, sx, sy, bw);
}

function drawUnit(a) {
    const sx = a.x * cellPx - camX, sy = a.y * cellPx - camY;
    if (sx + cellPx*2 < 0 || sx - cellPx > canvas.width || sy + cellPx*2 < 0 || sy - cellPx > canvas.height) return;

    const scale = cellPx / CELL_PX;

    // Check for husk sprite (destroyed units)
    const huskName = (a.hp <= 0) ? HUSK_SPRITES[a.actor_type] : null;
    const spriteName = huskName || a.actor_type;
    const info = spriteInfo[spriteName];

    if (info && spriteImages[spriteName]) {
        const drawW = info.width * scale, drawH = info.height * scale;
        const cx = sx + cellPx/2 - drawW/2, cy = sy + cellPx/2 - drawH/2;

        // Facing: vehicles/ships/aircraft have 32 facings, infantry 8
        let frame = 0;
        const isVehicle = a.kind === 'Vehicle' || a.kind === 'Mcv' || a.kind === 'Aircraft' || a.kind === 'Ship';
        if (!huskName) {
            if (isVehicle && info.frames >= 32) {
                const step = 1024 / 32;
                frame = Math.floor(((a.facing + step/2) & 1023) / step) % 32;
            } else if (a.kind === 'Infantry' && info.frames >= 8) {
                const step = 1024 / 8;
                frame = Math.floor(((a.facing + step/2) & 1023) / step) % 8;
            }
        }

        if (drawSprite(spriteName, frame, cx, cy, drawW, drawH, a.owner)) {
            // Ship turret overlay
            const turretName = SHIP_TURRETS[a.actor_type];
            if (turretName && !huskName) {
                const tInfo = spriteInfo[turretName];
                if (tInfo && spriteImages[turretName] && tInfo.frames >= 32) {
                    const tW = tInfo.width * scale, tH = tInfo.height * scale;
                    const step = 1024 / 32;
                    const tFrame = Math.floor(((a.facing + step/2) & 1023) / step) % 32;
                    const tCx = sx + cellPx/2 - tW/2, tCy = sy + cellPx/2 - tH/2;
                    drawSprite(turretName, tFrame, tCx, tCy, tW, tH, a.owner);
                }
            }
            // Helicopter rotor overlay (animated)
            const rotorName = ROTOR_SPRITES[a.actor_type];
            if (rotorName && !huskName) {
                const rInfo = spriteInfo[rotorName];
                if (rInfo && spriteImages[rotorName]) {
                    const rW = rInfo.width * scale, rH = rInfo.height * scale;
                    const rFrame = (currentTick * 2) % rInfo.frames;
                    const rCx = sx + cellPx/2 - rW/2, rCy = sy + cellPx/2 - rH/2;
                    const rotorFrames = spriteImages[rotorName];
                    if (rotorFrames[rFrame]) {
                        ctx.drawImage(rotorFrames[rFrame], rCx, rCy, rW, rH);
                    }
                }
            }
            drawHealthBar(a, sx, sy, cellPx);
            return;
        }
    }
    // Fallback: simple shapes
    const color = PLAYER_COLORS[a.owner] || '#888';
    const cx = sx + cellPx/2, cy = sy + cellPx/2;
    const r = Math.max(3, cellPx * 0.3);
    ctx.fillStyle = color;
    if (a.kind === 'Infantry') {
        ctx.beginPath(); ctx.arc(cx, cy, r*0.5, 0, Math.PI*2); ctx.fill();
        ctx.strokeStyle = '#000'; ctx.lineWidth = 1; ctx.stroke();
    } else {
        ctx.fillRect(cx-r, cy-r*0.6, r*2, r*1.2);
        ctx.strokeStyle = '#000'; ctx.lineWidth = 1;
        ctx.strokeRect(cx-r, cy-r*0.6, r*2, r*1.2);
    }
    if (cellPx >= 14) {
        ctx.fillStyle = '#fff'; ctx.font = `${Math.max(7, cellPx*0.4)}px monospace`;
        ctx.textAlign = 'center'; ctx.fillText(a.actor_type, cx, cy + 3);
    }
    drawHealthBar(a, sx, sy, cellPx);
}

function drawTree(a) {
    const sx = a.x * cellPx - camX, sy = a.y * cellPx - camY;
    if (sx + cellPx < 0 || sx - cellPx > canvas.width) return;
    // Try sprite lookup: tree actor_type maps to ter:<type>.tem
    const temName = `ter:${a.actor_type}.tem`;
    const frames = spriteImages[temName];
    const info = spriteInfo[temName];
    if (frames && frames[0] && info) {
        const scale = cellPx / CELL_PX;
        const drawW = info.width * scale;
        const drawH = info.height * scale;
        const cx = sx + cellPx / 2 - drawW / 2;
        const cy = sy + cellPx / 2 - drawH / 2;
        ctx.drawImage(frames[0], cx, cy, drawW, drawH);
        return;
    }
    // Geometric fallback
    const cx = sx + cellPx/2, cy = sy + cellPx/2;
    const s = Math.max(3, cellPx * 0.4);
    ctx.fillStyle = '#3a2a1a';
    ctx.fillRect(cx - s*0.15, cy, s*0.3, s*0.6);
    ctx.fillStyle = '#1a4a1a';
    ctx.beginPath(); ctx.arc(cx, cy - s*0.2, s, 0, Math.PI*2); ctx.fill();
    ctx.fillStyle = '#2a5a22';
    ctx.beginPath(); ctx.arc(cx - s*0.15, cy - s*0.4, s*0.7, 0, Math.PI*2); ctx.fill();
    ctx.fillStyle = '#1a3a16';
    ctx.beginPath(); ctx.arc(cx + s*0.2, cy - s*0.1, s*0.5, 0, Math.PI*2); ctx.fill();
}

function drawMine(a) {
    const sx = a.x * cellPx - camX, sy = a.y * cellPx - camY;
    if (sx + cellPx < 0 || sx - cellPx > canvas.width) return;
    // Try sprite lookup
    const temName = `ter:${a.actor_type}.tem`;
    const frames = spriteImages[temName];
    const info = spriteInfo[temName];
    if (frames && frames[0] && info) {
        const scale = cellPx / CELL_PX;
        const drawW = info.width * scale;
        const drawH = info.height * scale;
        const cx = sx + cellPx / 2 - drawW / 2;
        const cy = sy + cellPx / 2 - drawH / 2;
        ctx.drawImage(frames[0], cx, cy, drawW, drawH);
        return;
    }
    // Geometric fallback
    const cx = sx + cellPx/2, cy = sy + cellPx/2;
    const r = Math.max(2, cellPx * 0.2);
    ctx.fillStyle = '#5a4a30';
    ctx.beginPath(); ctx.ellipse(cx, cy, r, r*0.6, 0, 0, Math.PI*2); ctx.fill();
    ctx.strokeStyle = '#3a2a10'; ctx.lineWidth = 1; ctx.stroke();
}

// ── OpenRA-style health bar: 3 lines above the actor bounds ──
// x,y = top-left of actor footprint, w = footprint width in pixels
function drawHealthBar(a, x, y, w) {
    if (a.max_hp <= 0 || a.hp >= a.max_hp) return;
    const ratio = a.hp / a.max_hp;
    const barW = w;
    const fillW = Math.round(barW * ratio);
    // Bar sits above the actor: 3 lines at y-6, y-5, y-4
    const barY = y - 6;

    // Health color by damage state (OpenRA thresholds)
    let r, g, b;
    if (ratio <= 0.25) { r = 255; g = 0; b = 0; }       // Critical: Red
    else if (ratio <= 0.5) { r = 255; g = 255; b = 0; }  // Heavy: Yellow
    else { r = 0; g = 255; b = 0; }                       // Normal: LimeGreen

    // Background (dark)
    ctx.fillStyle = 'rgba(0,0,0,0.6)';
    ctx.fillRect(x, barY, barW, 3);

    // Line 1 (top): dimmed
    ctx.fillStyle = `rgb(${r>>1},${g>>1},${b>>1})`;
    ctx.fillRect(x, barY, fillW, 1);
    // Line 2 (middle): full brightness
    ctx.fillStyle = `rgb(${r},${g},${b})`;
    ctx.fillRect(x, barY + 1, fillW, 1);
    // Line 3 (bottom): dimmed
    ctx.fillStyle = `rgb(${r>>1},${g>>1},${b>>1})`;
    ctx.fillRect(x, barY + 2, fillW, 1);
}

// ── OpenRA-style selection brackets (white corner marks) ──
function drawSelectionBrackets(a) {
    let sx, sy, bw, bh;
    if (a.kind === 'Building') {
        const fp = BUILDING_FOOTPRINTS[a.actor_type] || [2,2];
        sx = a.x * cellPx - camX; sy = a.y * cellPx - camY;
        bw = fp[0] * cellPx; bh = fp[1] * cellPx;
    } else {
        // Use cell bounds for units
        sx = a.x * cellPx - camX; sy = a.y * cellPx - camY;
        bw = cellPx; bh = cellPx;
    }

    const L = Math.max(4, Math.min(bw, bh) * 0.25);
    ctx.strokeStyle = '#fff';
    ctx.lineWidth = 1;
    ctx.beginPath();
    // Top-left
    ctx.moveTo(sx + L, sy); ctx.lineTo(sx, sy); ctx.lineTo(sx, sy + L);
    // Top-right
    ctx.moveTo(sx + bw - L, sy); ctx.lineTo(sx + bw, sy); ctx.lineTo(sx + bw, sy + L);
    // Bottom-right
    ctx.moveTo(sx + bw, sy + bh - L); ctx.lineTo(sx + bw, sy + bh); ctx.lineTo(sx + bw - L, sy + bh);
    // Bottom-left
    ctx.moveTo(sx + L, sy + bh); ctx.lineTo(sx, sy + bh); ctx.lineTo(sx, sy + bh - L);
    ctx.stroke();

    // Health bar for selected actors (always shown)
    drawHealthBar(a, sx, sy, bw);
}

function drawPlacementGhost() {
    const [fw, fh] = placementMode.footprint;
    const gx = mouseCell.x * cellPx - camX, gy = mouseCell.y * cellPx - camY;
    const ok = session?.can_place_building?.(placementMode.type, mouseCell.x, mouseCell.y) ?? false;
    ctx.fillStyle = ok ? 'rgba(68,180,68,0.25)' : 'rgba(180,68,68,0.25)';
    ctx.fillRect(gx, gy, fw * cellPx, fh * cellPx);
    ctx.strokeStyle = ok ? '#4a8a2a' : '#8a2a2a';
    ctx.lineWidth = 1;
    ctx.setLineDash([3, 3]); ctx.strokeRect(gx, gy, fw * cellPx, fh * cellPx); ctx.setLineDash([]);
}

// ── Minimap (drawn in sidebar canvas) ──
function drawMinimap(snapshot) {
    const mmW = 222, mmH = 222;
    mmCtx.fillStyle = '#000';
    mmCtx.fillRect(0, 0, mmW, mmH);

    const scale = Math.min(mmW / mapW, mmH / mapH);
    // Center the minimap
    const offX = (mmW - mapW * scale) / 2;
    const offY = (mmH - mapH * scale) / 2;

    // Terrain thumbnail
    if (terrainCanvas) {
        mmCtx.drawImage(terrainCanvas, offX, offY, mapW * scale, mapH * scale);
    }

    // Resources (small dots)
    if (snapshot.resources) {
        for (const res of snapshot.resources) {
            mmCtx.fillStyle = res.kind === 1 ? '#a08020' : '#8040c0';
            mmCtx.fillRect(offX + res.x * scale, offY + res.y * scale, Math.max(1, scale), Math.max(1, scale));
        }
    }

    // Actors
    for (const a of snapshot.actors) {
        if (a.kind === 'Tree' || a.kind === 'Mine') continue;
        mmCtx.fillStyle = PLAYER_COLORS[a.owner] || '#888';
        if (a.kind === 'Building') {
            const fp = BUILDING_FOOTPRINTS[a.actor_type] || [2,2];
            mmCtx.fillRect(offX + a.x * scale, offY + a.y * scale,
                Math.max(2, fp[0]*scale), Math.max(2, fp[1]*scale));
        } else {
            mmCtx.fillRect(offX + a.x * scale, offY + a.y * scale,
                Math.max(1, scale*1.5), Math.max(1, scale*1.5));
        }
    }

    // Viewport rectangle (white outline)
    mmCtx.strokeStyle = '#fff';
    mmCtx.lineWidth = 1;
    mmCtx.strokeRect(
        offX + camX / cellPx * scale,
        offY + camY / cellPx * scale,
        canvas.width / cellPx * scale,
        canvas.height / cellPx * scale
    );
}

// ── Init ──
await init();
await loadSprites();
showScreen('home');
