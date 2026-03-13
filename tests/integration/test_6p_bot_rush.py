"""3 rush bots vs 3 rush bots on the most complex map (Pressure, 128x128, 8 spawns).

All 6 players are AI bots on Hard difficulty. We fast-forward ticks and take
periodic screenshots to observe the battle.
"""
import os
import time
from helpers import ORA

SCREENSHOT_DIR = os.path.join(os.path.dirname(__file__), "screenshots", "6p_rush")
os.makedirs(SCREENSHOT_DIR, exist_ok=True)

# Map 7 = Pressure (128x128, 8 spawns) - most complex bundled map
MAP_INDEX = 7
DIFFICULTY = 2  # Hard (rush)
NUM_PLAYERS = 6
TICKS_PER_STEP = 300
MAX_TICKS = 12000
SCREENSHOT_EVERY = 2  # every 600 ticks


def test_6p_bot_rush(page, server):
    """Run 3v3 (6-player FFA) rush bots on the Pressure map."""
    page.set_viewport_size({"width": 1280, "height": 720})
    page.goto(server, wait_until="networkidle")
    page.wait_for_function(
        "!document.querySelector('#map-select option').textContent.includes('Initializing')",
        timeout=60000,
    )

    # Start 6-player bot FFA game
    page.evaluate(
        f"window._ora.startBotFFA({MAP_INDEX}, {DIFFICULTY}, {NUM_PLAYERS})"
    )
    page.wait_for_function(f"window._ora.currentTick > 0", timeout=15000)

    # Stop the auto game loop so we control ticking
    page.evaluate("window._ora_pause_loop = true")

    # Disable fog so we can see the full map
    page.evaluate("window._ora.setFog(false)")

    # Center camera on the map
    page.evaluate("""(() => {
        const o = window._ora;
        const snap = JSON.parse(o.session.snapshot_json());
        o.render(snap);
        o.updateHUD(snap);
    })()""")

    page.screenshot(path=os.path.join(SCREENSHOT_DIR, "000_start.png"))
    print(f"\n6-PLAYER RUSH BOT GAME — Map: Pressure (128x128), Difficulty: Hard")
    print(f"{'='*70}")

    t0 = time.time()
    total_ticks = 0
    step = 0
    winner = 0

    while total_ticks < MAX_TICKS and winner == 0:
        result = page.evaluate(f"""(() => {{
            const s = window._ora.session;
            return s.tick_n({TICKS_PER_STEP});
        }})()""")
        total_ticks += TICKS_PER_STEP
        step += 1
        winner = result

        if step % SCREENSHOT_EVERY == 0 or winner > 0:
            state = page.evaluate("""(() => {
                const o = window._ora;
                const snap = JSON.parse(o.session.snapshot_json());
                o.render(snap);
                o.updateHUD(snap);
                const actors = snap.actors || [];
                const players = snap.players || [];
                const result = [];
                for (const p of players) {
                    const pa = actors.filter(a => a.owner === p.index);
                    const buildings = pa.filter(a => a.kind === 'Building').length;
                    const units = pa.filter(a =>
                        a.kind === 'Vehicle' || a.kind === 'Infantry'
                    ).length;
                    if (buildings > 0 || units > 0) {
                        result.push({
                            idx: p.index,
                            cash: p.cash,
                            buildings,
                            units,
                        });
                    }
                }
                return result;
            })()""")

            summary_parts = []
            for p in state:
                summary_parts.append(
                    f"P{p['idx']}:{p['buildings']}b/{p['units']}u/${p['cash']}"
                )
            print(f"  tick {total_ticks:5d}: {' | '.join(summary_parts)}")

            page.wait_for_timeout(50)
            fname = f"{step:03d}_tick{total_ticks}.png"
            if winner > 0:
                fname = f"{step:03d}_tick{total_ticks}_winner{winner}.png"
            page.screenshot(path=os.path.join(SCREENSHOT_DIR, fname))

    elapsed = time.time() - t0

    # Final render + screenshot
    page.evaluate("""(() => {
        const o = window._ora;
        const snap = JSON.parse(o.session.snapshot_json());
        o.render(snap);
        o.updateHUD(snap);
    })()""")
    page.wait_for_timeout(50)
    page.screenshot(path=os.path.join(SCREENSHOT_DIR, f"final_tick{total_ticks}.png"))

    # Final summary
    summary = page.evaluate("""(() => {
        const snap = JSON.parse(window._ora.session.snapshot_json());
        const players = snap.players || [];
        const actors = snap.actors || [];
        return {
            winner: window._ora.session.winner(),
            tick: window._ora.session.current_frame(),
            players: players.map(p => ({
                index: p.index,
                cash: p.cash,
                buildings: actors.filter(a => a.owner === p.index && a.kind === 'Building').length,
                units: actors.filter(a => a.owner === p.index && (a.kind === 'Vehicle' || a.kind === 'Infantry')).length,
            })),
        };
    })()""")

    print(f"\n{'='*70}")
    winner_label = f"Player {winner}" if winner > 0 else "DRAW / ONGOING"
    print(f"Result: {winner_label} — {total_ticks} ticks in {elapsed:.1f}s")
    for p in summary.get("players", []):
        if p.get("buildings", 0) > 0 or p.get("units", 0) > 0:
            print(f"  Player {p['index']}: ${p.get('cash', 0)}, "
                  f"{p.get('buildings', 0)} buildings, {p.get('units', 0)} units")
    print(f"\nScreenshots saved to: {SCREENSHOT_DIR}")

    # Assert the game ran successfully (at least some ticks processed)
    assert total_ticks > 0, "Game should have processed some ticks"
    assert len(summary.get("players", [])) >= NUM_PLAYERS, \
        f"Should have {NUM_PLAYERS} players, got {len(summary.get('players', []))}"
