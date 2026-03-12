//! Test player color remapping on sprites.

use openra_data::{mix, shp, palette};

#[test]
fn render_player_colored_units() {
    let mix_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../vendor/ra-content/");
    let conquer_data = std::fs::read(format!("{}conquer.mix", mix_dir)).unwrap();
    let conquer = mix::MixArchive::parse(conquer_data).unwrap();

    let pal_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../vendor/OpenRA/mods/ra/maps/chernobyl/temperat.pal"
    );
    let pal = palette::Palette::from_bytes(&std::fs::read(pal_path).unwrap()).unwrap();

    // Player colors (Blue and Red)
    let player_colors: [(u8, u8, u8); 2] = [(68, 136, 221), (221, 68, 68)];

    fn rgb_to_hsv(r: u8, g: u8, b: u8) -> (f32, f32, f32) {
        let r = r as f32 / 255.0;
        let g = g as f32 / 255.0;
        let b = b as f32 / 255.0;
        let mx = r.max(g).max(b);
        let mn = r.min(g).min(b);
        let d = mx - mn;
        let s = if mx == 0.0 { 0.0 } else { d / mx };
        let h = if d == 0.0 {
            0.0
        } else if mx == r {
            ((g - b) / d + 6.0) % 6.0 / 6.0
        } else if mx == g {
            ((b - r) / d + 2.0) / 6.0
        } else {
            ((r - g) / d + 4.0) / 6.0
        };
        (h, s, mx)
    }

    fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
        let i = (h * 6.0).floor() as u32;
        let f = h * 6.0 - i as f32;
        let p = v * (1.0 - s);
        let q = v * (1.0 - f * s);
        let t = v * (1.0 - (1.0 - f) * s);
        let (r, g, b) = match i % 6 {
            0 => (v, t, p),
            1 => (q, v, p),
            2 => (p, v, t),
            3 => (p, q, v),
            4 => (t, p, v),
            _ => (v, p, q),
        };
        ((r * 255.0) as u8, (g * 255.0) as u8, (b * 255.0) as u8)
    }

    let units = ["1tnk", "2tnk", "mcv", "harv"];
    let img_w = units.len() * 60;
    let img_h = 2 * 60; // 2 rows for 2 players
    let mut pixels = vec![[40u8, 70, 40]; img_w * img_h]; // Dark green bg

    for (pi, &(pr, pg, pb)) in player_colors.iter().enumerate() {
        let (ph, ps, _) = rgb_to_hsv(pr, pg, pb);
        for (ui, &name) in units.iter().enumerate() {
            let shp_data = conquer.get(&format!("{}.shp", name)).unwrap();
            let shp_file = shp::decode(shp_data).unwrap();
            let frame = &shp_file.frames[0];

            let ox = ui * 60 + (60 - frame.width as usize) / 2;
            let oy = pi * 60 + (60 - frame.height as usize) / 2;

            for sy in 0..frame.height as usize {
                for sx in 0..frame.width as usize {
                    let pi_idx = sy * frame.width as usize + sx;
                    let pal_idx = frame.pixels[pi_idx];
                    if pal_idx == 0 { continue; }

                    let (r, g, b) = if pal_idx >= 80 && pal_idx <= 95 {
                        // Remap: preserve brightness, apply player hue+saturation
                        let c = pal.colors[pal_idx as usize];
                        let (_, _, orig_v) = rgb_to_hsv(c[0], c[1], c[2]);
                        hsv_to_rgb(ph, ps, orig_v)
                    } else {
                        let c = pal.colors[pal_idx as usize];
                        (c[0], c[1], c[2])
                    };

                    let dx = ox + sx;
                    let dy = oy + sy;
                    if dx < img_w && dy < img_h {
                        pixels[dy * img_w + dx] = [r, g, b];
                    }
                }
            }
        }
    }

    let mut ppm = format!("P3\n{} {}\n255\n", img_w, img_h);
    for [r, g, b] in &pixels {
        ppm += &format!("{} {} {} ", r, g, b);
    }

    let out_path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../vendor/extracted-sprites/player_colors.ppm"
    );
    std::fs::write(out_path, &ppm).unwrap();
    println!("Wrote player color test to {}", out_path);
}
