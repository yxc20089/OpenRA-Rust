import init, { ReplayViewer } from './pkg/openra_wasm.js';

const canvas = document.getElementById('canvas');
const ctx = canvas.getContext('2d');
const status = document.getElementById('status');
const btnLoad = document.getElementById('btn-load');
const btnPlay = document.getElementById('btn-play');
const btnStep = document.getElementById('btn-step');
const speedSlider = document.getElementById('speed');
const speedVal = document.getElementById('speed-val');
const replayInput = document.getElementById('replay-file');
const mapInput = document.getElementById('map-file');

let viewer = null;
let playing = false;
let animFrameId = null;
let replayBytes = null;
let mapBytes = null;
let lastSnapshot = null;

// Player colors (index by player actor ID)
const PLAYER_COLORS = [
    '#888888', // 0: neutral/world (grey)
    '#888888', // 1: Neutral (grey)
    '#888888', // 2: Creeps (grey)
    '#ffcc00', // 3: Player 1 (yellow)
    '#e94560', // 4: Player 2 (red)
    '#4488ff', // 5: Everyone (blue)
    '#44cc44', // 6: extra green
    '#cc44cc', // 7: extra purple
];

// Building footprint sizes (cells)
const BUILDING_FOOTPRINTS = {
    'fact': [3, 2], 'weap': [3, 2], 'weap.ukraine': [3, 2], 'proc': [3, 2],
    'fix': [3, 2], 'spen': [3, 3], 'syrd': [3, 3],
    'powr': [2, 2], 'apwr': [2, 2], 'tent': [2, 2], 'barr': [2, 2],
    'dome': [2, 2], 'hpad': [2, 2], 'afld': [2, 2], 'atek': [2, 2], 'stek': [2, 2],
    'tsla': [1, 1], 'sam': [1, 1], 'gap': [1, 1], 'agun': [1, 1],
    'pbox': [1, 1], 'hbox': [1, 1], 'gun': [1, 1], 'ftur': [1, 1],
};

// Unit type symbols for minimap rendering
const UNIT_SYMBOLS = {
    'e1': 'R', 'e2': 'G', 'e3': 'B', 'e4': 'F', 'e6': 'E', 'e7': 'T',
    'shok': 'S', 'medi': '+', 'mech': 'W', 'dog': 'D', 'spy': '?', 'thf': '$',
    '1tnk': '1', '2tnk': '2', '3tnk': '3', '4tnk': '4',
    'v2rl': 'V', 'arty': 'A', 'harv': 'H', 'mcv': 'M',
    'apc': 'P', 'jeep': 'J', 'mnly': 'L', 'ttnk': 'T', 'ctnk': 'C',
    'heli': '^', 'hind': '^', 'mig': '>', 'yak': '>',
};

function getPlayerColor(playerIndex) {
    return PLAYER_COLORS[playerIndex] || '#ffffff';
}

// Enable load button when both files are selected
function checkFiles() {
    btnLoad.disabled = !(replayBytes && mapBytes);
}

replayInput.addEventListener('change', async (e) => {
    const file = e.target.files[0];
    if (file) {
        replayBytes = new Uint8Array(await file.arrayBuffer());
        status.textContent = `Replay loaded: ${file.name} (${replayBytes.length} bytes)`;
    }
    checkFiles();
});

mapInput.addEventListener('change', async (e) => {
    const file = e.target.files[0];
    if (file) {
        mapBytes = new Uint8Array(await file.arrayBuffer());
        status.textContent = `Map loaded: ${file.name} (${mapBytes.length} bytes)`;
    }
    checkFiles();
});

btnLoad.addEventListener('click', () => {
    try {
        viewer = new ReplayViewer(replayBytes, mapBytes);
        status.textContent = `Loaded! ${viewer.total_frames()} frames. Use Play or Step.`;
        btnPlay.disabled = false;
        btnStep.disabled = false;
        lastSnapshot = JSON.parse(viewer.snapshot_json());
        render(lastSnapshot);
    } catch (e) {
        status.textContent = `Error: ${e}`;
    }
});

btnPlay.addEventListener('click', () => {
    if (playing) {
        playing = false;
        btnPlay.textContent = 'Play';
        if (animFrameId) cancelAnimationFrame(animFrameId);
    } else {
        playing = true;
        btnPlay.textContent = 'Pause';
        runLoop();
    }
});

btnStep.addEventListener('click', () => {
    if (!viewer) return;
    stepOnce();
});

speedSlider.addEventListener('input', () => {
    speedVal.textContent = speedSlider.value;
});

function stepOnce() {
    const ok = viewer.tick();
    if (!ok) {
        playing = false;
        btnPlay.textContent = 'Play';
        btnPlay.disabled = true;
        btnStep.disabled = true;
        status.textContent = 'Replay finished.';
        return false;
    }
    lastSnapshot = JSON.parse(viewer.snapshot_json());
    render(lastSnapshot);

    // Count actors by type
    const buildings = lastSnapshot.actors.filter(a => a.kind === 'Building').length;
    const units = lastSnapshot.actors.filter(a =>
        a.kind === 'Infantry' || a.kind === 'Vehicle' || a.kind === 'Mcv').length;

    status.textContent = `Frame ${viewer.current_frame()} / ${viewer.total_frames()} | ` +
        `Tick ${lastSnapshot.tick} | ${buildings} buildings, ${units} units`;
    return true;
}

function runLoop() {
    if (!playing || !viewer) return;
    const framesPerRaf = parseInt(speedSlider.value);
    for (let i = 0; i < framesPerRaf; i++) {
        if (!stepOnce()) return;
    }
    animFrameId = requestAnimationFrame(runLoop);
}

function render(snapshot) {
    const w = canvas.width;
    const h = canvas.height;

    // Dark terrain background
    ctx.fillStyle = '#1a3a1a';
    ctx.fillRect(0, 0, w, h);

    if (!snapshot) return;

    // Map cell coords to canvas pixels
    const mapW = snapshot.map_width || 128;
    const mapH = snapshot.map_height || 128;
    const scaleX = w / mapW;
    const scaleY = h / mapH;
    const scale = Math.min(scaleX, scaleY);
    const offsetX = (w - mapW * scale) / 2;
    const offsetY = (h - mapH * scale) / 2;

    // Draw grid lines (subtle)
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

    // === Layer 1: Resources ===
    if (snapshot.resources) {
        for (const res of snapshot.resources) {
            const rx = offsetX + res.x * scale;
            const ry = offsetY + res.y * scale;
            const alpha = 0.3 + 0.05 * res.density;
            if (res.kind === 1) {
                // Ore: yellow-brown
                ctx.fillStyle = `rgba(180, 140, 40, ${alpha})`;
            } else if (res.kind === 2) {
                // Gems: purple
                ctx.fillStyle = `rgba(160, 60, 200, ${alpha})`;
            }
            ctx.fillRect(rx, ry, scale, scale);
        }
    }

    // === Layer 2: Trees and Mines (terrain decorations) ===
    for (const actor of snapshot.actors) {
        const sx = offsetX + actor.x * scale + scale / 2;
        const sy = offsetY + actor.y * scale + scale / 2;

        if (actor.kind === 'Tree') {
            ctx.fillStyle = '#2d5a2d';
            const size = Math.max(2, scale * 0.7);
            ctx.fillRect(sx - size / 2, sy - size / 2, size, size);
            // Tree crown
            ctx.fillStyle = '#3d7a3d';
            const crown = size * 0.5;
            ctx.fillRect(sx - crown / 2, sy - size / 2 - crown / 2, crown, crown);
        } else if (actor.kind === 'Mine') {
            ctx.fillStyle = '#cc8833';
            const size = Math.max(2, scale * 0.5);
            ctx.save();
            ctx.translate(sx, sy);
            ctx.rotate(Math.PI / 4);
            ctx.fillRect(-size / 2, -size / 2, size, size);
            ctx.restore();
        }
    }

    // === Layer 3: Buildings ===
    for (const actor of snapshot.actors) {
        if (actor.kind !== 'Building') continue;
        const color = getPlayerColor(actor.owner);
        const fp = BUILDING_FOOTPRINTS[actor.actor_type] || [2, 2];
        const bx = offsetX + actor.x * scale;
        const by = offsetY + actor.y * scale;
        const bw = fp[0] * scale;
        const bh = fp[1] * scale;

        // Building body
        ctx.fillStyle = color;
        ctx.fillRect(bx + 1, by + 1, bw - 2, bh - 2);

        // Building outline
        ctx.strokeStyle = 'rgba(0,0,0,0.5)';
        ctx.lineWidth = 1;
        ctx.strokeRect(bx + 1, by + 1, bw - 2, bh - 2);

        // Building type label
        if (scale > 3 && actor.actor_type) {
            ctx.fillStyle = '#000';
            ctx.font = `${Math.max(7, scale * 0.7)}px monospace`;
            ctx.textAlign = 'center';
            ctx.fillText(actor.actor_type, bx + bw / 2, by + bh / 2 + scale * 0.25);
        }

        // Health bar (only if damaged)
        if (actor.max_hp > 0 && actor.hp < actor.max_hp) {
            drawHealthBar(bx, by - 3, bw, 2, actor.hp / actor.max_hp);
        }
    }

    // === Layer 4: Units ===
    for (const actor of snapshot.actors) {
        if (actor.kind !== 'Infantry' && actor.kind !== 'Vehicle' &&
            actor.kind !== 'Mcv' && actor.kind !== 'Aircraft' && actor.kind !== 'Ship') continue;

        const sx = offsetX + actor.x * scale + scale / 2;
        const sy = offsetY + actor.y * scale + scale / 2;
        const color = getPlayerColor(actor.owner);

        if (actor.kind === 'Infantry') {
            // Infantry: small circle
            const r = Math.max(2, scale * 0.3);
            ctx.fillStyle = color;
            ctx.beginPath();
            ctx.arc(sx, sy, r, 0, Math.PI * 2);
            ctx.fill();

            // Activity indicator
            if (actor.activity === 'attacking') {
                ctx.strokeStyle = '#ff0000';
                ctx.lineWidth = 1;
                ctx.stroke();
            }
        } else if (actor.kind === 'Vehicle' || actor.kind === 'Mcv') {
            // Vehicles: larger filled circle
            const r = Math.max(3, scale * 0.45);
            ctx.fillStyle = color;
            ctx.beginPath();
            ctx.arc(sx, sy, r, 0, Math.PI * 2);
            ctx.fill();

            // Outline
            ctx.strokeStyle = 'rgba(0,0,0,0.4)';
            ctx.lineWidth = 1;
            ctx.stroke();

            // Harvester special: show carried resources
            if (actor.actor_type === 'harv' && actor.activity === 'harvesting') {
                ctx.strokeStyle = '#ffff00';
                ctx.lineWidth = 1.5;
                ctx.stroke();
            }
        } else if (actor.kind === 'Aircraft') {
            // Aircraft: triangle
            const r = Math.max(3, scale * 0.4);
            ctx.fillStyle = color;
            ctx.beginPath();
            ctx.moveTo(sx, sy - r);
            ctx.lineTo(sx - r * 0.7, sy + r * 0.5);
            ctx.lineTo(sx + r * 0.7, sy + r * 0.5);
            ctx.closePath();
            ctx.fill();
        } else if (actor.kind === 'Ship') {
            // Ships: diamond
            const r = Math.max(3, scale * 0.4);
            ctx.fillStyle = color;
            ctx.beginPath();
            ctx.moveTo(sx, sy - r);
            ctx.lineTo(sx + r, sy);
            ctx.lineTo(sx, sy + r);
            ctx.lineTo(sx - r, sy);
            ctx.closePath();
            ctx.fill();
        }

        // Unit type symbol (when zoomed in enough)
        if (scale > 5) {
            const sym = UNIT_SYMBOLS[actor.actor_type] || '';
            if (sym) {
                ctx.fillStyle = '#000';
                ctx.font = `bold ${Math.max(6, scale * 0.4)}px monospace`;
                ctx.textAlign = 'center';
                ctx.fillText(sym, sx, sy + scale * 0.15);
            }
        }

        // Health bar (only if damaged)
        if (actor.max_hp > 0 && actor.hp < actor.max_hp) {
            const barW = Math.max(6, scale * 0.8);
            drawHealthBar(sx - barW / 2, sy - scale * 0.5 - 3, barW, 2, actor.hp / actor.max_hp);
        }
    }

    // === HUD: Player info panel ===
    ctx.textAlign = 'left';
    const panelX = 8;
    let panelY = 16;
    ctx.font = 'bold 13px monospace';

    for (const p of snapshot.players) {
        const color = getPlayerColor(p.index);
        ctx.fillStyle = color;

        let powerStr = '';
        if (p.power_provided > 0 || p.power_drained > 0) {
            const powerStatus = p.power_drained > p.power_provided ? ' LOW' : '';
            powerStr = ` | Power: ${p.power_provided}/${p.power_drained}${powerStatus}`;
        }

        ctx.fillText(`P${p.index}: $${p.cash}${powerStr}`, panelX, panelY);
        panelY += 18;
    }

    // Tick counter
    ctx.fillStyle = 'rgba(255,255,255,0.6)';
    ctx.font = '11px monospace';
    ctx.textAlign = 'right';
    ctx.fillText(`Tick ${snapshot.tick}`, w - 8, h - 8);
}

function drawHealthBar(x, y, w, h, ratio) {
    // Background
    ctx.fillStyle = 'rgba(0,0,0,0.6)';
    ctx.fillRect(x, y, w, h);
    // Health fill
    if (ratio > 0.5) {
        ctx.fillStyle = '#44cc44';
    } else if (ratio > 0.25) {
        ctx.fillStyle = '#cccc44';
    } else {
        ctx.fillStyle = '#cc4444';
    }
    ctx.fillRect(x, y, w * ratio, h);
}

// Initialize WASM
await init();
status.textContent = 'WASM loaded. Select replay and map files.';
