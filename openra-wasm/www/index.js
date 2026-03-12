import init, { ReplayViewer, GameSession } from './pkg/openra_wasm.js';

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
let mode = null; // 'game' | 'replay'
let session = null; // GameSession or ReplayViewer
let humanPlayerId = null;
let selectedUnits = []; // array of actor IDs
let lastSnapshot = null;
let buildableItems = null;
let placementMode = null; // { type, footprint: [w,h] } or null
let playing = false;
let animFrameId = null;
let mouseCell = { x: -1, y: -1 }; // current mouse cell position

// Rendering geometry (computed per frame)
let mapW = 128, mapH = 128, scale = 1, offsetX = 0, offsetY = 0;

// ── Player colors ──
const PLAYER_COLORS = {
    0: '#888', 1: '#888', 2: '#888',
    3: '#4488ff', 4: '#e94560', 5: '#888',
    6: '#44cc44', 7: '#cc44cc',
};

const BUILDING_FOOTPRINTS = {
    'fact': [3, 2], 'weap': [3, 2], 'weap.ukraine': [3, 2], 'proc': [3, 2],
    'fix': [3, 2], 'spen': [3, 3], 'syrd': [3, 3],
    'powr': [2, 2], 'apwr': [2, 2], 'tent': [2, 2], 'barr': [2, 2],
    'dome': [2, 2], 'hpad': [2, 2], 'afld': [2, 2], 'atek': [2, 2], 'stek': [2, 2],
    'tsla': [1, 1], 'sam': [1, 1], 'gap': [1, 1], 'agun': [1, 1],
    'pbox': [1, 1], 'hbox': [1, 1], 'gun': [1, 1], 'ftur': [1, 1],
};

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
    } catch (e) {
        replayStatus.textContent = `Error: ${e}`;
    }
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
    document.getElementById('speed').oninput = (e) => {
        document.getElementById('speed-val').textContent = e.target.value;
    };
}

function replayStep() {
    if (!session.tick()) {
        playing = false;
        hudMsg.textContent = 'Replay finished.';
        return false;
    }
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
        // Make human player color blue
        PLAYER_COLORS[humanPlayerId] = '#4488ff';
        PLAYER_COLORS[humanPlayerId + 1] = '#e94560'; // bot

        setupGameUI();
        showScreen('game');
        resizeCanvas();
        gameLoop();
    } catch (e) {
        alert(`Failed to start game: ${e}`);
    }
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
        const msg = winner === humanPlayerId ? 'You win!' : 'You lost!';
        hudMsg.textContent = msg;
        return;
    }
    // Refresh buildable every 30 frames
    if (session.current_frame() % 30 === 0) refreshBuildable();

    setTimeout(gameLoop, 40); // ~25 FPS
}

// ── Sidebar: buildable items ──
function refreshBuildable() {
    if (mode !== 'game') return;
    try {
        buildableItems = JSON.parse(session.buildable_items_json());
    } catch { buildableItems = []; }

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

// ── Sidebar: production queue ──
function refreshQueue(snapshot) {
    if (mode !== 'game' || !snapshot) return;
    const myPlayer = snapshot.players.find(p => p.index === humanPlayerId);
    if (!myPlayer || !myPlayer.production_queue || myPlayer.production_queue.length === 0) {
        queueSection.style.display = 'none';
        return;
    }
    queueSection.style.display = 'block';
    queueList.innerHTML = '';
    for (const item of myPlayer.production_queue) {
        const div = document.createElement('div');
        div.className = 'queue-item' + (item.done ? ' done' : '');
        const pct = Math.round(item.progress * 100);
        div.innerHTML = `${item.item_name} ${pct}%<div class="bar"><div class="bar-fill" style="width:${pct}%"></div></div>`;
        if (item.done) {
            // If it's a building, click to enter placement mode
            const buildInfo = buildableItems?.find(b => b.name === item.item_name && b.is_building);
            if (buildInfo) {
                div.style.cursor = 'pointer';
                div.style.color = '#44cc44';
                div.innerHTML += ' <small>(click map to place)</small>';
                div.onclick = () => {
                    placementMode = { type: item.item_name, footprint: [buildInfo.footprint[0], buildInfo.footprint[1]] };
                    hudMsg.textContent = `Place ${item.item_name} — click on map (right-click to cancel)`;
                };
                // Auto-enter placement mode for first completed building
                if (!placementMode) {
                    placementMode = { type: item.item_name, footprint: [buildInfo.footprint[0], buildInfo.footprint[1]] };
                    hudMsg.textContent = `Place ${item.item_name} — click on map`;
                }
            }
        }
        queueList.appendChild(div);
    }
}

// ── Sidebar: selection info ──
function refreshSelection() {
    if (selectedUnits.length === 0 || !lastSnapshot) {
        selSection.style.display = 'none';
        return;
    }
    selSection.style.display = 'block';
    const actors = lastSnapshot.actors.filter(a => selectedUnits.includes(a.id));
    if (actors.length === 0) { selSection.style.display = 'none'; return; }

    if (actors.length === 1) {
        const a = actors[0];
        const hpPct = a.max_hp > 0 ? Math.round(a.hp / a.max_hp * 100) : 100;
        selInfo.innerHTML = `<span class="name">${a.actor_type || a.kind}</span><br>
            HP: ${a.hp}/${a.max_hp} (${hpPct}%)<br>
            Activity: ${a.activity}`;
    } else {
        selInfo.innerHTML = `<span class="name">${actors.length} units selected</span>`;
    }

    selActions.innerHTML = '';
    // Deploy button for MCVs
    if (actors.some(a => a.kind === 'Mcv')) {
        const btn = document.createElement('button');
        btn.className = 'action-btn';
        btn.textContent = 'Deploy';
        btn.onclick = () => {
            for (const a of actors) {
                if (a.kind === 'Mcv') session.order_deploy(a.id);
            }
        };
        selActions.appendChild(btn);
    }
    // Sell button for buildings
    if (actors.some(a => a.kind === 'Building')) {
        const btn = document.createElement('button');
        btn.className = 'action-btn';
        btn.textContent = 'Sell';
        btn.onclick = () => {
            for (const a of actors) {
                if (a.kind === 'Building') session.order_sell(a.id);
            }
        };
        selActions.appendChild(btn);
    }
    // Stop button
    const stopBtn = document.createElement('button');
    stopBtn.className = 'action-btn';
    stopBtn.textContent = 'Stop';
    stopBtn.onclick = () => { for (const a of actors) session.order_stop(a.id); };
    selActions.appendChild(stopBtn);
}

// ── HUD ──
function updateHUD(snapshot) {
    if (!snapshot) return;
    const myPlayer = snapshot.players.find(p => p.index === humanPlayerId);
    if (myPlayer) {
        hudCash.textContent = `$${myPlayer.cash}`;
        const low = myPlayer.power_drained > myPlayer.power_provided;
        hudPower.textContent = `Power: ${myPlayer.power_provided}/${myPlayer.power_drained}${low ? ' LOW' : ''}`;
        hudPower.style.color = low ? '#e94560' : '#e0e0e0';
    } else if (mode === 'replay') {
        // Show all players in replay mode
        const p = snapshot.players[0];
        if (p) hudCash.textContent = `P${p.index}: $${p.cash}`;
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

// ── Coordinate conversion ──
function computeGeometry(snapshot) {
    mapW = snapshot.map_width || 128;
    mapH = snapshot.map_height || 128;
    const scaleX = canvas.width / mapW;
    const scaleY = canvas.height / mapH;
    scale = Math.min(scaleX, scaleY);
    offsetX = (canvas.width - mapW * scale) / 2;
    offsetY = (canvas.height - mapH * scale) / 2;
}

function canvasToCell(px, py) {
    return {
        x: Math.floor((px - offsetX) / scale),
        y: Math.floor((py - offsetY) / scale),
    };
}

function actorAtCell(cx, cy, snapshot) {
    // Check buildings first (footprint-aware)
    for (const a of snapshot.actors) {
        if (a.kind === 'Building') {
            const fp = BUILDING_FOOTPRINTS[a.actor_type] || [2, 2];
            if (cx >= a.x && cx < a.x + fp[0] && cy >= a.y && cy < a.y + fp[1]) return a;
        }
    }
    // Then units
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
    // Re-render if in placement mode to show ghost
    if (placementMode && lastSnapshot) render(lastSnapshot);
});

canvas.addEventListener('click', (e) => {
    if (!lastSnapshot) return;
    const rect = canvas.getBoundingClientRect();
    const cell = canvasToCell(e.clientX - rect.left, e.clientY - rect.top);

    if (mode === 'game') {
        handleGameClick(cell, e.shiftKey);
    } else if (mode === 'replay') {
        // In replay, left-click selects for info only
        const actor = actorAtCell(cell.x, cell.y, lastSnapshot);
        if (actor) {
            selectedUnits = [actor.id];
        } else {
            selectedUnits = [];
        }
        refreshSelection();
    }
});

canvas.addEventListener('contextmenu', (e) => {
    e.preventDefault();
    if (mode !== 'game' || !lastSnapshot) return;
    const rect = canvas.getBoundingClientRect();
    const cell = canvasToCell(e.clientX - rect.left, e.clientY - rect.top);

    if (placementMode) {
        placementMode = null;
        hudMsg.textContent = '';
        return;
    }

    if (selectedUnits.length === 0) return;

    // Right-click: contextual action
    const target = actorAtCell(cell.x, cell.y, lastSnapshot);
    if (target && target.owner !== humanPlayerId && target.owner > 2) {
        // Attack enemy
        for (const uid of selectedUnits) {
            session.order_attack(uid, target.id);
        }
        hudMsg.textContent = `Attacking ${target.actor_type || target.kind}`;
    } else {
        // Move to cell
        for (const uid of selectedUnits) {
            session.order_move(uid, cell.x, cell.y);
        }
        hudMsg.textContent = `Moving to (${cell.x}, ${cell.y})`;
    }
});

function handleGameClick(cell, shiftKey) {
    // Building placement mode
    if (placementMode) {
        if (session.can_place_building(placementMode.type, cell.x, cell.y)) {
            session.order_place_building(placementMode.type, cell.x, cell.y);
            hudMsg.textContent = `Placed ${placementMode.type}`;
            placementMode = null;
            refreshBuildable();
        } else {
            hudMsg.textContent = 'Cannot place here!';
        }
        return;
    }

    // Click on actor
    const actor = actorAtCell(cell.x, cell.y, lastSnapshot);
    if (actor && actor.owner === humanPlayerId) {
        if (shiftKey) {
            // Add to selection
            if (!selectedUnits.includes(actor.id)) selectedUnits.push(actor.id);
        } else {
            selectedUnits = [actor.id];
        }
    } else if (actor && actor.owner !== humanPlayerId && actor.owner > 2 && selectedUnits.length > 0) {
        // Clicked enemy with units selected — attack
        for (const uid of selectedUnits) {
            session.order_attack(uid, actor.id);
        }
        hudMsg.textContent = `Attacking ${actor.actor_type || actor.kind}`;
    } else {
        if (!shiftKey) selectedUnits = [];
    }
    refreshSelection();
}

// ── Drag select ──
let dragStart = null;
canvas.addEventListener('mousedown', (e) => {
    if (e.button !== 0) return;
    const rect = canvas.getBoundingClientRect();
    dragStart = { x: e.clientX - rect.left, y: e.clientY - rect.top };
});

canvas.addEventListener('mouseup', (e) => {
    if (e.button !== 0 || !dragStart || !lastSnapshot || mode !== 'game') {
        dragStart = null;
        return;
    }
    const rect = canvas.getBoundingClientRect();
    const dragEnd = { x: e.clientX - rect.left, y: e.clientY - rect.top };
    const dx = Math.abs(dragEnd.x - dragStart.x);
    const dy = Math.abs(dragEnd.y - dragStart.y);

    // Only treat as drag if moved more than 10 pixels
    if (dx > 10 || dy > 10) {
        const c1 = canvasToCell(Math.min(dragStart.x, dragEnd.x), Math.min(dragStart.y, dragEnd.y));
        const c2 = canvasToCell(Math.max(dragStart.x, dragEnd.x), Math.max(dragStart.y, dragEnd.y));
        selectedUnits = [];
        for (const a of lastSnapshot.actors) {
            if (a.owner !== humanPlayerId) continue;
            if (a.kind === 'Building' || a.kind === 'Tree' || a.kind === 'Mine') continue;
            if (a.x >= c1.x && a.x <= c2.x && a.y >= c1.y && a.y <= c2.y) {
                selectedUnits.push(a.id);
            }
        }
        refreshSelection();
    }
    dragStart = null;
});

// ── Keyboard shortcuts ──
document.addEventListener('keydown', (e) => {
    if (mode !== 'game') return;
    if (e.key === 'Escape') {
        if (placementMode) { placementMode = null; hudMsg.textContent = ''; }
        else { selectedUnits = []; refreshSelection(); }
    }
    if (e.key === 's' || e.key === 'S') {
        for (const uid of selectedUnits) session.order_stop(uid);
    }
    if (e.key === 'd' || e.key === 'D') {
        // Deploy selected MCVs
        if (!lastSnapshot) return;
        for (const uid of selectedUnits) {
            const a = lastSnapshot.actors.find(a => a.id === uid);
            if (a && a.kind === 'Mcv') session.order_deploy(uid);
        }
    }
});

// ── Render ──
function getColor(owner) { return PLAYER_COLORS[owner] || '#fff'; }

function render(snapshot) {
    const w = canvas.width, h = canvas.height;
    computeGeometry(snapshot);

    // Background
    ctx.fillStyle = '#1a3a1a';
    ctx.fillRect(0, 0, w, h);
    if (!snapshot) return;

    // Grid lines
    if (scale > 4) {
        ctx.strokeStyle = 'rgba(255,255,255,0.03)';
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
    }

    // Resources
    if (snapshot.resources) {
        for (const res of snapshot.resources) {
            const rx = offsetX + res.x * scale;
            const ry = offsetY + res.y * scale;
            const alpha = 0.3 + 0.05 * res.density;
            ctx.fillStyle = res.kind === 1
                ? `rgba(180,140,40,${alpha})`
                : `rgba(160,60,200,${alpha})`;
            ctx.fillRect(rx, ry, scale, scale);
        }
    }

    // Trees and mines
    for (const a of snapshot.actors) {
        const sx = offsetX + a.x * scale + scale / 2;
        const sy = offsetY + a.y * scale + scale / 2;
        if (a.kind === 'Tree') {
            ctx.fillStyle = '#2d5a2d';
            const s = Math.max(2, scale * 0.7);
            ctx.fillRect(sx - s / 2, sy - s / 2, s, s);
            ctx.fillStyle = '#3d7a3d';
            const c = s * 0.5;
            ctx.fillRect(sx - c / 2, sy - s / 2 - c / 2, c, c);
        } else if (a.kind === 'Mine') {
            ctx.fillStyle = '#cc8833';
            const s = Math.max(2, scale * 0.5);
            ctx.save(); ctx.translate(sx, sy); ctx.rotate(Math.PI / 4);
            ctx.fillRect(-s / 2, -s / 2, s, s);
            ctx.restore();
        }
    }

    // Buildings
    for (const a of snapshot.actors) {
        if (a.kind !== 'Building') continue;
        const color = getColor(a.owner);
        const fp = BUILDING_FOOTPRINTS[a.actor_type] || [2, 2];
        const bx = offsetX + a.x * scale;
        const by = offsetY + a.y * scale;
        const bw = fp[0] * scale;
        const bh = fp[1] * scale;

        ctx.fillStyle = color;
        ctx.fillRect(bx + 1, by + 1, bw - 2, bh - 2);
        ctx.strokeStyle = 'rgba(0,0,0,0.5)';
        ctx.lineWidth = 1;
        ctx.strokeRect(bx + 1, by + 1, bw - 2, bh - 2);

        // Label
        if (scale > 3 && a.actor_type) {
            ctx.fillStyle = '#000';
            ctx.font = `${Math.max(7, scale * 0.7)}px monospace`;
            ctx.textAlign = 'center';
            ctx.fillText(a.actor_type, bx + bw / 2, by + bh / 2 + scale * 0.25);
        }

        // Selection highlight
        if (selectedUnits.includes(a.id)) {
            ctx.strokeStyle = '#44ff44';
            ctx.lineWidth = 2;
            ctx.strokeRect(bx, by, bw, bh);
        }

        // Health bar
        if (a.max_hp > 0 && a.hp < a.max_hp) {
            drawHealthBar(bx, by - 4, bw, 3, a.hp / a.max_hp);
        }
    }

    // Units
    for (const a of snapshot.actors) {
        if (!['Infantry', 'Vehicle', 'Mcv', 'Aircraft', 'Ship'].includes(a.kind)) continue;
        const sx = offsetX + a.x * scale + scale / 2;
        const sy = offsetY + a.y * scale + scale / 2;
        const color = getColor(a.owner);
        const selected = selectedUnits.includes(a.id);

        if (a.kind === 'Infantry') {
            const r = Math.max(2, scale * 0.3);
            ctx.fillStyle = color;
            ctx.beginPath(); ctx.arc(sx, sy, r, 0, Math.PI * 2); ctx.fill();
            if (a.activity === 'attacking') {
                ctx.strokeStyle = '#ff0000'; ctx.lineWidth = 1; ctx.stroke();
            }
        } else if (a.kind === 'Vehicle' || a.kind === 'Mcv') {
            const r = Math.max(3, scale * 0.45);
            ctx.fillStyle = color;
            ctx.beginPath(); ctx.arc(sx, sy, r, 0, Math.PI * 2); ctx.fill();
            ctx.strokeStyle = 'rgba(0,0,0,0.4)'; ctx.lineWidth = 1; ctx.stroke();
            if (a.actor_type === 'harv' && a.activity === 'harvesting') {
                ctx.strokeStyle = '#ffff00'; ctx.lineWidth = 1.5; ctx.stroke();
            }
        } else if (a.kind === 'Aircraft') {
            const r = Math.max(3, scale * 0.4);
            ctx.fillStyle = color;
            ctx.beginPath(); ctx.moveTo(sx, sy - r); ctx.lineTo(sx - r * 0.7, sy + r * 0.5);
            ctx.lineTo(sx + r * 0.7, sy + r * 0.5); ctx.closePath(); ctx.fill();
        } else if (a.kind === 'Ship') {
            const r = Math.max(3, scale * 0.4);
            ctx.fillStyle = color;
            ctx.beginPath(); ctx.moveTo(sx, sy - r); ctx.lineTo(sx + r, sy);
            ctx.lineTo(sx, sy + r); ctx.lineTo(sx - r, sy); ctx.closePath(); ctx.fill();
        }

        // Selection ring
        if (selected) {
            const r = Math.max(4, scale * 0.5);
            ctx.strokeStyle = '#44ff44';
            ctx.lineWidth = 2;
            ctx.beginPath(); ctx.arc(sx, sy, r, 0, Math.PI * 2); ctx.stroke();
        }

        // Unit type label
        if (scale > 5 && a.actor_type) {
            ctx.fillStyle = '#000';
            ctx.font = `bold ${Math.max(6, scale * 0.4)}px monospace`;
            ctx.textAlign = 'center';
            ctx.fillText(a.actor_type, sx, sy + scale * 0.15);
        }

        // Health bar
        if (a.max_hp > 0 && a.hp < a.max_hp) {
            const bw = Math.max(6, scale * 0.8);
            drawHealthBar(sx - bw / 2, sy - scale * 0.5 - 4, bw, 3, a.hp / a.max_hp);
        }
    }

    // Placement ghost
    if (placementMode && mouseCell.x >= 0 && mouseCell.y >= 0) {
        const [fw, fh] = placementMode.footprint;
        const gx = offsetX + mouseCell.x * scale;
        const gy = offsetY + mouseCell.y * scale;
        const canPlace = session && session.can_place_building
            ? session.can_place_building(placementMode.type, mouseCell.x, mouseCell.y)
            : false;
        ctx.fillStyle = canPlace ? 'rgba(68,204,68,0.3)' : 'rgba(204,68,68,0.3)';
        ctx.fillRect(gx, gy, fw * scale, fh * scale);
        ctx.strokeStyle = canPlace ? '#44cc44' : '#cc4444';
        ctx.lineWidth = 2;
        ctx.strokeRect(gx, gy, fw * scale, fh * scale);
        if (scale > 3) {
            ctx.fillStyle = '#fff';
            ctx.font = `${Math.max(8, scale * 0.6)}px monospace`;
            ctx.textAlign = 'center';
            ctx.fillText(placementMode.type, gx + fw * scale / 2, gy + fh * scale / 2 + scale * 0.2);
        }
    }

    // Player info overlay (replay mode)
    if (mode === 'replay') {
        ctx.textAlign = 'left';
        let py = 16;
        ctx.font = 'bold 13px monospace';
        for (const p of snapshot.players) {
            ctx.fillStyle = getColor(p.index);
            let powerStr = '';
            if (p.power_provided > 0 || p.power_drained > 0) {
                const low = p.power_drained > p.power_provided;
                powerStr = ` | Power: ${p.power_provided}/${p.power_drained}${low ? ' LOW' : ''}`;
            }
            ctx.fillText(`P${p.index}: $${p.cash}${powerStr}`, 8, py);
            py += 18;
        }
    }
}

function drawHealthBar(x, y, w, h, ratio) {
    ctx.fillStyle = 'rgba(0,0,0,0.6)';
    ctx.fillRect(x, y, w, h);
    ctx.fillStyle = ratio > 0.5 ? '#44cc44' : ratio > 0.25 ? '#cccc44' : '#cc4444';
    ctx.fillRect(x, y, w * ratio, h);
}

// ── Init ──
await init();
showScreen('home');
