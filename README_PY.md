# openra_train (Python bindings)

PyO3 bindings for the OpenRA-Rust training runtime.

## Build

```bash
pip install maturin
maturin develop --release
```

This compiles `openra-train` with the `python` feature enabled and
installs `openra_train` into the active virtualenv.

## Usage

```python
import openra_train

env = openra_train.OpenRAEnv("rush-hour", 42)
obs = env.reset()
print(obs["unit_positions"])

cmd = openra_train.Command.move_units(["1001"], target_x=60, target_y=20)
obs, reward, done, info = env.step([cmd])
```
