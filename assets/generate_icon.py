"""Generate the original Ferrum rook icon at high resolution (requires Pillow)."""
from pathlib import Path
from PIL import Image, ImageDraw

root = Path(__file__).resolve().parent
size = 1024
im = Image.new('RGBA', (size, size), (0, 0, 0, 0))
d = ImageDraw.Draw(im)
d.rounded_rectangle((32, 32, 992, 992), radius=200, fill='#1a2926', outline='#9dbb8b', width=18)
d.rounded_rectangle((75, 75, 949, 949), radius=160, outline='#344c42', width=8)
# A simple, original rook silhouette with crenellations and a substantial base.
d.polygon([(276,260),(360,260),(360,330),(452,330),(452,260),(572,260),(572,330),
           (664,330),(664,260),(748,260),(748,394),(686,458),(650,697),
           (711,757),(711,813),(313,813),(313,757),(374,697),(338,458),(276,394)],
          fill='#eceddb')
d.polygon([(351,425),(673,425),(650,455),(374,455)], fill='#9dbb8b')
d.rounded_rectangle((320, 730, 704, 771), radius=9, fill='#9dbb8b')
icon = im.resize((256, 256), Image.Resampling.LANCZOS)
icon.save(root / 'ferrum-icon.png')
icon.save(root / 'ferrum-icon.ico', sizes=[(16,16),(24,24),(32,32),(48,48),(64,64),(128,128),(256,256)])
(root / 'ferrum-icon.rgba').write_bytes(im.resize((64,64), Image.Resampling.LANCZOS).tobytes())
