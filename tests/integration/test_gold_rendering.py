"""Quick test to verify gold/ore overlay rendering."""
import os
from helpers import ORA, start_game, get_snapshot, wait_ticks, find_actors

SCREENSHOT_DIR = os.path.join(os.path.dirname(__file__), "screenshots")

def test_gold_rendering(game_page):
    """Start a game and screenshot the ore area to verify gold sprites."""
    page = game_page
    pid = start_game(page)
    snap = get_snapshot(page)

    # Find mines
    mines = find_actors(snap, kind="Mine")
    print(f"Found {len(mines)} mines")

    # Check resources in snapshot
    resources = snap.get("resources", [])
    print(f"Resource tiles: {len(resources)}")
    if resources:
        ore = [r for r in resources if r["kind"] == 1]
        gems = [r for r in resources if r["kind"] == 2]
        print(f"  Ore tiles: {len(ore)}, Gem tiles: {len(gems)}")
        if ore:
            print(f"  Sample ore: density={ore[0]['density']}, pos=({ore[0]['x']},{ore[0]['y']})")

    # Pan camera to ore area
    if mines:
        mine = mines[0]
        page.evaluate(f"""(() => {{
            const o = window._ora;
            const c = document.getElementById('canvas');
            // Center camera on mine
            const cellPx = o.cellPx;
            // Set camera via internal state by using a move
        }})()""")

        # Take screenshot of current view (should show base + nearby ore)
        page.screenshot(path=os.path.join(SCREENSHOT_DIR, "gold_rendering_base.png"))

        # Check if gold sprites are in the sprite atlas info
        gold_sprites = page.evaluate(f"""(() => {{
            const info = {ORA}.spriteInfo || {{}};
            return Object.keys(info).filter(k => k.includes('gold') || k.includes('gem'));
        }})()""")
        print(f"Gold/gem sprites in atlas: {gold_sprites}")
        total_sprites = page.evaluate(f"Object.keys({ORA}.spriteInfo || {{}}).length")
        print(f"Total sprites loaded: {total_sprites}")

        # Check which mix files contain the gold .tem files
        mix_check = page.evaluate("""(async () => {
            const mod = await import('./pkg/openra_wasm.js');
            return JSON.parse(mod.SpriteAtlas.check_mix_files([
                'gold01.tem', 'gold02.tem', 'gem01.tem',
                'GOLD01.TEM', 'gold01.shp',
                'mine.tem', 't01.tem'
            ]));
        })()""")
        print(f"Mix file check: {mix_check}")

    # Wait and take more screenshots
    wait_ticks(page, 50)
    page.screenshot(path=os.path.join(SCREENSHOT_DIR, "gold_rendering_50ticks.png"))
    print(f"Screenshots saved to {SCREENSHOT_DIR}")
