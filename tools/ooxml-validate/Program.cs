// Validates .pptx files with the Open XML SDK, the schema checks Office
// itself is closest to. Usage: ooxml-validate <file-or-directory>...
// Exits with 1 if any file has errors.

using DocumentFormat.OpenXml;
using DocumentFormat.OpenXml.Packaging;
using DocumentFormat.OpenXml.Validation;

var files = args
    .SelectMany(a => Directory.Exists(a)
        ? Directory.GetFiles(a, "*.pptx", SearchOption.AllDirectories)
        : new[] { a })
    .OrderBy(f => f)
    .ToList();

var validator = new OpenXmlValidator(FileFormatVersions.Microsoft365);
var failed = 0;
foreach (var file in files)
{
    List<ValidationErrorInfo> errors;
    try
    {
        using var doc = PresentationDocument.Open(file, false);
        errors = validator.Validate(doc).ToList();
    }
    catch (Exception e)
    {
        Console.WriteLine($"{file}: cannot open: {e.Message}");
        failed++;
        continue;
    }
    if (errors.Count == 0)
    {
        continue;
    }
    failed++;
    Console.WriteLine($"{file}: {errors.Count} error(s)");
    foreach (var e in errors.Take(10))
    {
        Console.WriteLine($"  {e.Part?.Uri} {e.Path?.XPath}");
        Console.WriteLine($"    {e.Description}");
    }
}

Console.WriteLine($"{files.Count} file(s) checked, {failed} with errors");
return failed == 0 ? 0 : 1;
