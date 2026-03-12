"""Run 10 medium-bot vs medium-bot games, observing via Playwright screenshots.

Both players are bots. We fast-forward ticks and take periodic screenshots.
"""
import os
import time
from helpers import ORA

SCREENSHOT_DIR = os.path.join(os.path.dirname(__file__), "screenshots", "bot_vs_bot")
os.makedirs(SCREENSHOT_DIR, exist_ok=True)

MAP_COUNT = 8
TICKS_PER_STEP = 200
MAX_TICKS = 8000
SCREENSHOT_EVERY = 2  # every 400 ticks


def run_bot_game(page, server_url, game_num, map_index=0, difficulty=1):
    """Run a single bot-vs-bot game. Returns result dict."""
    game_dir = os.path.join(SCREENSHOT_DIR, f"game_{game_num:02d}")
    os.makedirs(game_dir, exist_ok=True)

    page.set_viewport_size({"width": 1280, "height": 720})
    page.goto(server_url, wait_until="networkidle")
    page.wait_for_function(
        "!document.querySelector('#map-select option').textContent.includes('Initializing')",
        timeout=60000,
    )

    # Start bot-vs-bot game (async - returns promise)
    page.evaluate(f"window._ora.startBotVsBot({map_index}, {difficulty})")
    page.wait_for_function(f"window._ora.currentTick > 0", timeout=15000)

    # Stop the auto game loop so we control ticking
    page.evaluate("window._ora_pause_loop = true")

    page.screenshot(path=os.path.join(game_dir, "000_start.png"))

    t0 = time.time()
    total_ticks = 0
    step = 0
    winner = 0

    while total_ticks < MAX_TICKS and winner == 0:
        # Fast-forward N ticks via tick_n
        result = page.evaluate(f"""(() => {{
            const s = window._ora.session;
            const w = s.tick_n({TICKS_PER_STEP});
            return w;
        }})()""")
        total_ticks += TICKS_PER_STEP
        step += 1
        winner = result

        if step % SCREENSHOT_EVERY == 0 or winner > 0:
            state = page.evaluate(f"""(() => {{
                const o = window._ora;
                const snap = JSON.parse(o.session.snapshot_json());
                o.render(snap);
                o.updateHUD(snap);
                const actors = snap.actors || [];
                const p3 = actors.filter(a => a.owner === 3);
                const p4 = actors.filter(a => a.owner === 4);
                const p3units = p3.filter(a => a.kind === 'Vehicle' || a.kind === 'Infantry');
                const p4units = p4.filter(a => a.kind === 'Vehicle' || a.kind === 'Infantry');
                return {{
                    p3_buildings: p3.filter(a => a.kind === 'Building').length,
                    p3_units: p3units.length,
                    p3_activities: p3units.slice(0, 3).map(u => u.activity),
                    p4_buildings: p4.filter(a => a.kind === 'Building').length,
                    p4_units: p4units.length,
                    p3_cash: (snap.players || []).find(p => p.index === 3)?.cash || 0,
                    p4_cash: (snap.players || []).find(p => p.index === 4)?.cash || 0,
                }};
            }})()""")
            print(f"  tick {total_ticks}: P3={state['p3_buildings']}b/{state['p3_units']}u/${state['p3_cash']} P4={state['p4_buildings']}b/{state['p4_units']}u/${state['p4_cash']} activities={state.get('p3_activities', [])}")
            page.wait_for_timeout(50)
            fname = f"{step:03d}_tick{total_ticks}.png"
            if winner > 0:
                fname = f"{step:03d}_tick{total_ticks}_winner{winner}.png"
            page.screenshot(path=os.path.join(game_dir, fname))

    elapsed = time.time() - t0

    # Final render + screenshot
    page.evaluate(f"""(() => {{
        const o = window._ora;
        const snap = JSON.parse(o.session.snapshot_json());
        o.render(snap);
        o.updateHUD(snap);
    }})()""")
    page.wait_for_timeout(50)
    page.screenshot(path=os.path.join(game_dir, f"final_tick{total_ticks}.png"))

    summary = page.evaluate(f"""(() => {{
        const snap = JSON.parse(window._ora.session.snapshot_json());
        const players = snap.players || [];
        const actors = snap.actors || [];
        return {{
            winner: window._ora.session.winner(),
            tick: window._ora.session.current_frame(),
            players: players.map(p => ({{
                index: p.index,
                cash: p.cash,
                buildings: actors.filter(a => a.owner === p.index && a.kind === 'Building').length,
                units: actors.filter(a => a.owner === p.index && (a.kind === 'Vehicle' || a.kind === 'Infantry')).length,
            }})),
        }};
    }})()""")

    return {
        "game": game_num,
        "map_index": map_index,
        "winner": winner,
        "ticks": total_ticks,
        "elapsed_sec": round(elapsed, 1),
        "summary": summary,
    }


def test_bot_vs_bot_10_games(page, server):
    """Run 10 medium-bot vs medium-bot games across different maps."""
    results = []

    for game_num in range(10):
        map_idx = game_num % MAP_COUNT
        print(f"\n{'='*60}")
        print(f"GAME {game_num + 1}/10 — Map index {map_idx}, Medium difficulty")
        print(f"{'='*60}")

        result = run_bot_game(page, server, game_num + 1, map_index=map_idx, difficulty=1)
        results.append(result)

        winner_label = f"Player {result['winner']}" if result["winner"] > 0 else "DRAW"
        print(f"  Result: {winner_label} wins in {result['ticks']} ticks ({result['elapsed_sec']}s)")
        for p in result.get("summary", {}).get("players", []):
            if p.get("buildings", 0) > 0 or p.get("units", 0) > 0:
                print(f"    Player {p['index']}: ${p.get('cash',0)}, {p.get('buildings',0)} buildings, {p.get('units',0)} units")

    # Summary
    print(f"\n{'='*60}")
    print("OVERALL RESULTS")
    print(f"{'='*60}")
    wins = {}
    for r in results:
        w = r["winner"]
        wins[w] = wins.get(w, 0) + 1
    for w, count in sorted(wins.items()):
        label = f"Player {w}" if w > 0 else "Draw"
        print(f"  {label}: {count} wins")

    avg_ticks = sum(r["ticks"] for r in results) / len(results)
    avg_time = sum(r["elapsed_sec"] for r in results) / len(results)
    print(f"  Average game length: {avg_ticks:.0f} ticks ({avg_time:.1f}s)")
    print(f"\nScreenshots saved to: {SCREENSHOT_DIR}")
