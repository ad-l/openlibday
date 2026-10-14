import hashlib
from pathlib import Path
import struct
import tempfile
import unittest
import zipfile

import libday_mod as mod


from testsupport import GREEN_SKIRMISH, ROOT, TOOLS


class ModTests(unittest.TestCase):
    def test_exact_display_patch_and_preserved_original(self):
        source = ROOT / "EXE" / "LIBDAY.EXE"
        original = source.read_bytes()
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "patched.exe"
            result = mod.patch_display(source, output, 1920, 1080)
            patched = output.read_bytes()
            self.assertEqual(patched[:mod.MODE_OFFSET], original[:mod.MODE_OFFSET])
            self.assertEqual(patched[mod.MODE_OFFSET + 8:], original[mod.MODE_OFFSET + 8:])
            self.assertEqual(patched[mod.MODE_OFFSET:mod.MODE_OFFSET + 8], struct.pack("<II", 1920, 1080))
            self.assertEqual(result["source_sha256"], mod.ORIGINAL_SHA256)
            self.assertEqual(source.read_bytes(), original)
            with self.assertRaises(FileExistsError):
                mod.patch_display(source, output, 1920, 1080)

    def test_display_rejects_unknown_build_and_invalid_dimensions(self):
        with tempfile.TemporaryDirectory() as directory:
            source = Path(directory) / "wrong.exe"
            source.write_bytes(b"MZ")
            output = Path(directory) / "out.exe"
            with self.assertRaises(ValueError):
                mod.patch_display(source, output, 1920, 1080)
            with self.assertRaises(ValueError):
                mod.patch_display(ROOT / "EXE" / "LIBDAY.EXE", output, 1921, 1080)
            self.assertFalse(output.exists())

    def test_extract_and_add_custom_member_preserve_archive(self):
        archive = ROOT / "SCENS" / "DEATHM.ZIP"
        original_hash = hashlib.sha256(archive.read_bytes()).digest()
        with tempfile.TemporaryDirectory() as directory:
            database = Path(directory) / "custom.mdb"
            output = Path(directory) / "custom.zip"
            mod.extract_map(archive, "DMA2-10.MDB", database)
            mod.pack_map(archive, "custom.mdb", database, output)
            with zipfile.ZipFile(archive) as source, zipfile.ZipFile(output) as target:
                self.assertIsNone(target.testzip())
                self.assertEqual(target.read("custom.mdb"), source.read("dma2-10.mdb"))
                for item in source.infolist():
                    self.assertEqual(target.read(item.filename), source.read(item))
            self.assertEqual(hashlib.sha256(archive.read_bytes()).digest(), original_hash)

    def test_replace_and_reject_unsafe_or_invalid_inputs(self):
        with tempfile.TemporaryDirectory() as directory:
            archive = Path(directory) / "template.zip"
            database = Path(directory) / "input.mdb"
            output = Path(directory) / "output.zip"
            old = mod.JET_HEADER + b"old"
            new = mod.JET_HEADER + b"new"
            database.write_bytes(new)
            with zipfile.ZipFile(archive, "w") as target:
                target.writestr("test.mdb", old, compress_type=zipfile.ZIP_DEFLATED)
            mod.pack_map(archive, "TEST.MDB", database, output)
            with zipfile.ZipFile(output) as target:
                self.assertEqual(target.namelist(), ["test.mdb"])
                self.assertEqual(target.read("test.mdb"), new)
            for name in ["../test.mdb", "x/y.mdb", "toolongfilename.mdb"]:
                with self.assertRaises(ValueError):
                    mod.extract_map(archive, name, Path(directory) / "unsafe.mdb")
            database.write_bytes(b"not a database")
            with self.assertRaises(ValueError):
                mod.pack_map(archive, "test.mdb", database, Path(directory) / "invalid.zip")

    def test_staging_separates_writable_paths_and_keeps_original_binary(self):
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "playtest"
            windows_path = r"F:\test\playtest"
            mod.stage_game(ROOT, output, windows_path)
            ini = (output / "EXE" / "LIBDAY.INI").read_text(encoding="cp1252")
            self.assertIn("scenarios=F:\\test\\playtest\\SCENS", ini)
            self.assertIn("globals=F:\\test\\playtest\\GLOBALS", ini)
            self.assertIn("savegame=F:\\test\\playtest\\SAVEGAME", ini)
            self.assertIn("Resolution=1", ini)
            self.assertEqual((output / "EXE" / "LIBDAY.EXE").read_bytes(),
                             (ROOT / "EXE" / "LIBDAY.EXE").read_bytes())
            self.assertEqual((output / "SCENS" / "DEATHM.MDB").read_bytes(),
                             (ROOT / "SCENS" / "DEATHM.MDB").read_bytes())
            for archive in (ROOT / "SCENS").glob("*.ZIP"):
                cache = output / "CACHE" / archive.stem
                self.assertTrue(cache.is_dir())
                self.assertEqual(list(cache.iterdir()), [])
            with self.assertRaises(FileExistsError):
                mod.stage_game(ROOT, output, windows_path)


if __name__ == "__main__":
    unittest.main()
