import json
import shutil
import tempfile
import unittest
from pathlib import Path

from testsupport import ROOT  # noqa: F401  (skips without a game install)

from asset_editor_core import ACTIONS, AssetStore, capabilities, compose
from map_editor_core import Texture, TerrainArt


class AssetStoreTest(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.store = AssetStore(ROOT)

    @classmethod
    def tearDownClass(cls):
        cls.store.close()

    def setUp(self):
        self.temp = Path(tempfile.mkdtemp(prefix="asset-test-"))

    def tearDown(self):
        shutil.rmtree(self.temp, ignore_errors=True)

    def test_catalogue(self):
        store = self.store
        self.assertEqual(len(store.units), 62)
        self.assertEqual(len(store.buildings), 81)
        commando = store.units[0]
        self.assertEqual((commando.name, commando.image_id), ("Commando", "HCOM"))
        self.assertTrue(set(commando.levels) <= set(range(11)))
        self.assertEqual(store.labels["Movement"][2], "trooper")
        caps = capabilities(commando.levels[2], store.labels)
        self.assertIn("Stealth (Jammer 0)", caps)
        self.assertIn("Overwatch", caps)
        artillery = store.units[15]
        self.assertIn("Indirect fire", capabilities(artillery.base, store.labels))
        for kind in store.units.values():
            self.assertTrue(store.sprite_sets(kind.image_id), kind.name)
            self.assertTrue(store.unit_animations(kind), kind.name)
        for kind in store.buildings.values():
            self.assertTrue(store.sprite_sets(kind.image_id), kind.name)

    def test_unit_animation_letters_match_definitions(self):
        for kind in self.store.units.values():
            letters = {a.action for a in self.store.unit_animations(kind)}
            for letter, (_, column) in ACTIONS.items():
                if column == "ShootProjectileAnimation_Begin":
                    continue  # two types ship projectile art they do not use
                self.assertEqual(letter in letters, kind.base[column] != -1, (kind.name, letter))

    def test_compose_draws_back_to_front(self):
        red = Texture(54, 56, bytes([255, 0, 0, 255]) * (54 * 56))
        blue = Texture(54, 56, bytes([0, 0, 255, 255]) * (54 * 56))
        image = compose({(0, 0): red, (1, 0): blue}, 2, 1)
        self.assertEqual((image.width, image.height), (81, 70))
        # The overlap belongs to the later (front) part.
        pixel = ((20 * image.width) + 40) * 4
        self.assertEqual(image.rgba[pixel:pixel + 4], bytes([0, 0, 255, 255]))

    def test_sprite_round_trip_and_save(self):
        store = AssetStore(ROOT)
        try:
            name = store.sprite_sets("HCOM")[0, 0][0, 0]
            store.export_sprites([name], self.temp)
            png = self.temp / f"{name}.png"
            self.assertTrue(png.is_file())
            before = store.texture(name)
            # Re-importing the unchanged export must be lossless.
            store.replace_sprites({name: png})
            self.assertEqual(store.texture(name), before)
            self.assertEqual(store.sprite_edits, {name.upper()})
            # Paint the whole cell with an opaque palette colour.
            colour = store.art.palette[17]
            filled = Texture(54, 56, colour * (54 * 56))
            png.write_bytes(filled.png())
            store.replace_sprites({name: png})
            self.assertEqual(store.texture(name).rgba, filled.rgba)
            written = store.save_copies(self.temp / "out")
            self.assertEqual([p.name for p in written], ["UB.DBI"])
            self.assertEqual(TerrainArt(written[0]).texture(name).rgba, filled.rgba)
            other = store.sprite_sets("HCOM")[0, 1][0, 0]
            self.assertEqual(TerrainArt(written[0]).texture(other), TerrainArt(store.dbi_path).texture(other))
        finally:
            store.close()

    def test_animation_round_trip_and_install(self):
        store = AssetStore(ROOT)
        try:
            animation = store.unit_animations(store.units[0])[0]
            paths, meta = store.frames(animation.member)
            self.assertEqual(len(paths), meta["frames"])
            self.assertEqual((meta["width"], meta["height"]), (216, 224))
            frames = self.temp / "frames"
            store.export_animation(animation.member, frames)
            original = [p.read_bytes() for p in sorted(frames.glob("frame_*.png"))]
            # Swap the first two frames, re-encode and decode again.
            first, second = frames / "frame_000.png", frames / "frame_001.png"
            first.write_bytes(original[1])
            second.write_bytes(original[0])
            store.replace_animation(animation.member, frames)
            self.assertIn(animation.member, store.animation_edits)
            paths, _ = store.frames(animation.member)
            self.assertEqual(paths[0].read_bytes(), original[1])
            self.assertEqual(paths[1].read_bytes(), original[0])
            self.assertEqual(paths[2].read_bytes(), original[2])
            with self.assertRaises(RuntimeError):
                (frames / "frame_002.png").unlink()
                store.replace_animation(animation.member, frames)

            # Install into a scratch copy of the game files.
            game = self.temp / "game"
            game.mkdir()
            store.ff_path = game / "ANMSUNIT.FF"
            store.dbi_path = game / "UB.DBI"
            shutil.copyfile(AssetStoreTest.store.ff_path, store.ff_path)
            installed = store.install()
            self.assertEqual(installed, [store.ff_path])
            self.assertTrue((game / "ANMSUNIT.FF.orig").is_file())
            self.assertFalse(store.dbi_path.exists())
            exported = self.temp / "installed"
            store.cli("smk-export", f"{store.ff_path}!{animation.member}", exported)
            self.assertEqual((exported / "frame_000.png").read_bytes(), original[1])
            meta = json.loads((exported / "meta.json").read_text())
            self.assertEqual(meta["frames"], len(original))
            store.revert_animation(animation.member)
            self.assertFalse(store.dirty)
        finally:
            store.close()


if __name__ == "__main__":
    unittest.main()
