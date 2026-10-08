// Validates .pptx files with the Open XML SDK, the schema checks Office
// itself is closest to.
//
//   ooxml-validate [--tsv] <file-or-directory>...
//
// Prints a readable report, or with --tsv one line per error:
//   file <TAB> part <TAB> error id <TAB> description
// Exits with 1 if any file has errors or cannot be opened.

using DocumentFormat.OpenXml;
using DocumentFormat.OpenXml.Packaging;
using DocumentFormat.OpenXml.Validation;

var tsv = args.Contains("--tsv");
var files = args
    .Where(a => a != "--tsv")
    .SelectMany(a => Directory.Exists(a)
        ? Directory.GetFiles(a, "*.pptx", SearchOption.AllDirectories)
        : new[] { a })
    .OrderBy(f => f)
    .ToList();

var validator = new OpenXmlValidator(FileFormatVersions.Microsoft365);
var failed = 0;
foreach (var file in files)
{
    // Collect everything while the package is open; error objects point
    // back into it.
    var errors = new List<(string Part, string Id, string Path, string Description)>();
    try
    {
        using var doc = PresentationDocument.Open(file, false);
        foreach (var e in validator.Validate(doc))
        {
            errors.Add((e.Part?.Uri.ToString() ?? "", e.Id, e.Path?.XPath ?? "", e.Description));
        }
    }
    catch (Exception e)
    {
        errors.Add(("", "open", "", e.Message));
    }
    if (errors.Count == 0)
    {
        continue;
    }
    failed++;
    if (tsv)
    {
        foreach (var e in errors)
        {
            Console.WriteLine($"{file}\t{e.Part}\t{e.Id}\t{e.Description.ReplaceLineEndings(" ")}");
        }
        continue;
    }
    Console.WriteLine($"{file}: {errors.Count} error(s)");
    foreach (var e in errors.Take(10))
    {
        Console.WriteLine($"  {e.Part} {e.Path}");
        Console.WriteLine($"    {e.Description}");
    }
}

Console.Error.WriteLine($"{files.Count} file(s) checked, {failed} with errors");
return failed == 0 ? 0 : 1;
