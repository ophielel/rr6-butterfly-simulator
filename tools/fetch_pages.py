"""Fetch the fixed-content pages from wiki.gg into data/_raw/pages/."""
import os, sys
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import wikigg

PAGES = [
    # identities
    "Lobotomy E.G.O::Solemn Lament Yi Sang",
    "Lobotomy E.G.O::Lamp Gregor",
    "Lobotomy E.G.O::Faint Aroma & Solitude Ryōshū",
    "Jeong's Office Rep Ishmael",
    "Los Mariachis Jefe Sinclair",
    "LCA Udjat Vanguard Team 3 Leader Outis",
    "Lobotomy E.G.O::The Sword Sharpened with Tears Rodion",
    # E.G.O
    "Solemn Lament Yi Sang",
    "Bygone Days Yi Sang",
    "Solemn Lament Gregor",
    "Bygone Days Ishmael",
    "Tidal Elegy Ishmael",
    "Harmony Sinclair",
    "Rime Shank Rodion",
    # boss
    "Butterfly of Entangled Lives 羅生蝶",
    "Butterfly of Entangled Lives 羅生蝶/Enemy/Butterfly of Entangled Lives::Imago",
    "Line 6: Maru no Uchi no Sanzu no Kawa",
    # mechanics references
    "Status Effects",
    "Clash",
    "Damage",
]

out = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))), "data", "_raw", "pages")
os.makedirs(out, exist_ok=True)
for title in PAGES:
    fn = os.path.join(out, title.replace("/", "__").replace(" ", "_") + ".wikitext")
    if os.path.exists(fn) and os.path.getsize(fn) > 0:
        print("cached", title)
        continue
    try:
        text = wikigg.page(title)
    except Exception as exc:  # noqa: BLE001
        print("FAIL", title, exc)
        continue
    if text is None:
        print("MISSING", title)
        continue
    with open(fn, "w", encoding="utf-8") as fh:
        fh.write(text)
    print(f"ok {len(text):7d} {title}")
