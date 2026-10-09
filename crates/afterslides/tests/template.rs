use afterslides::{Categories, ChartData, Presentation, Series, ShapeKind, ShapeRef, XySeries};

const TEMPLATE: &[u8] = include_bytes!("../../../tests/fixtures/template.pptx");

fn open() -> Presentation {
    Presentation::from_bytes(TEMPLATE).expect("template opens")
}

fn reopen(prs: &mut Presentation) -> Presentation {
    let bytes = prs.to_bytes().expect("saves");
    Presentation::from_bytes(&bytes).expect("saved file opens")
}

fn shape(prs: &Presentation, slide: usize, name: &str) -> ShapeRef {
    let slide = prs.slide_at(slide).unwrap();
    prs.find_shape(slide, name)
        .unwrap()
        .unwrap_or_else(|| panic!("no shape {name:?}"))
}

fn part_names(prs: &Presentation) -> Vec<String> {
    prs.package().part_names().map(str::to_string).collect()
}

#[test]
fn untouched_round_trip_is_lossless() {
    let mut prs = open();
    let bytes = prs.to_bytes().unwrap();
    let mut a = zip::ZipArchive::new(std::io::Cursor::new(TEMPLATE)).unwrap();
    let mut b = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    assert_eq!(a.len(), b.len());
    for i in 0..a.len() {
        use std::io::Read;
        let mut fa = a.by_index(i).unwrap();
        let name = fa.name().to_string();
        let mut fb = b
            .by_name(&name)
            .unwrap_or_else(|_| panic!("{name} missing"));
        let (mut da, mut db) = (Vec::new(), Vec::new());
        fa.read_to_end(&mut da).unwrap();
        fb.read_to_end(&mut db).unwrap();
        assert!(da == db, "{name} changed");
    }
}

#[test]
fn lists_shapes() {
    let prs = open();
    assert_eq!(prs.slide_count().unwrap(), 7);
    let shapes = prs.shapes(prs.slide_at(5).unwrap()).unwrap();
    let names: Vec<_> = shapes.iter().map(|s| (s.name.as_str(), s.kind)).collect();
    assert_eq!(
        names,
        [
            ("Card", ShapeKind::Group),
            ("Grouped Label", ShapeKind::Shape),
            ("Logo", ShapeKind::Picture)
        ]
    );
    assert_eq!(shapes[1].parent, Some(shapes[0].shape.id));
    assert_eq!(shapes[2].alt_text, "logo:company");
}

#[test]
fn replaces_split_placeholders() {
    let mut prs = open();
    let n = prs
        .replace_text(&[
            ("{{title}}", "Q3 Report"),
            ("{{customer}}", "Acme"),
            ("{{period}}", "2026"),
            ("{{name}}", "Ada"),
        ])
        .unwrap();
    assert_eq!(n, 4);
    let prs = reopen(&mut prs);
    let subtitle = prs.shapes(prs.slide_at(0).unwrap()).unwrap()[1].shape;
    assert_eq!(
        prs.shape_text(subtitle).unwrap().as_deref(),
        Some("Report for Acme — 2026")
    );
    let label = shape(&prs, 5, "Grouped Label");
    assert_eq!(prs.shape_text(label).unwrap().as_deref(), Some("Hello Ada"));
}

#[test]
fn fills_category_chart_with_more_series() {
    let mut prs = open();
    let chart = shape(&prs, 1, "Revenue Chart");
    let data = ChartData::new(
        Categories::Labels(vec!["North".into(), "South".into(), "East".into()]),
        vec![
            Series::new("Plan", vec![Some(1.0), Some(2.0), Some(3.0)]),
            Series::new("Actual", vec![Some(1.5), None, Some(2.5)]),
            Series::new("Forecast", vec![Some(4.0), Some(5.0)]),
        ],
    );
    prs.set_chart_data(chart, &data).unwrap();
    let prs = reopen(&mut prs);
    let read = prs.chart_data(shape(&prs, 1, "Revenue Chart")).unwrap();
    assert_eq!(read.categories, data.categories);
    let values: Vec<_> = read
        .series
        .iter()
        .map(|s| (s.name.as_str(), s.values.clone()))
        .collect();
    assert_eq!(
        values,
        [
            ("Plan", vec![Some(1.0), Some(2.0), Some(3.0)]),
            ("Actual", vec![Some(1.5), None, Some(2.5)]),
            ("Forecast", vec![Some(4.0), Some(5.0), None]),
        ]
    );
}

#[test]
fn fills_chart_with_fewer_series() {
    let mut prs = open();
    let chart = shape(&prs, 1, "Revenue Chart");
    let data = ChartData::new(
        Categories::Labels(vec!["A".into()]),
        vec![Series::new("Only", vec![Some(42.0)])],
    );
    prs.set_chart_data(chart, &data).unwrap();
    let prs = reopen(&mut prs);
    let read = prs.chart_data(chart).unwrap();
    assert_eq!(read.categories, data.categories);
    assert_eq!(read.series.len(), 1);
    assert_eq!(read.series[0].values, [Some(42.0)]);
}

#[test]
fn fills_xy_charts() {
    let mut prs = open();
    let scatter = shape(&prs, 4, "Scatter");
    let series = vec![XySeries::new(
        "Run 1",
        vec![Some(0.0), Some(1.0)],
        vec![Some(1.0), Some(4.0)],
    )];
    prs.set_chart_xy_data(scatter, &series).unwrap();
    assert!(
        prs.set_chart_data(scatter, &ChartData::new(Categories::Labels(vec![]), vec![]),)
            .is_err()
    );

    let bubbles = shape(&prs, 4, "Bubbles");
    let bubble_series =
        vec![XySeries::new("M", vec![Some(1.0)], vec![Some(2.0)]).with_sizes(vec![Some(3.0)])];
    prs.set_chart_xy_data(bubbles, &bubble_series).unwrap();

    let prs = reopen(&mut prs);
    assert_eq!(prs.chart_xy_data(scatter).unwrap()[0].y, series[0].y);
    assert_eq!(prs.chart_xy_data(bubbles).unwrap(), bubble_series);
}

#[test]
fn chart_title() {
    let mut prs = open();
    let chart = shape(&prs, 1, "Revenue Chart");
    assert_eq!(prs.chart_title(chart).unwrap().as_deref(), Some("Revenue"));
    prs.set_chart_title(chart, "Umsatz").unwrap();
    let prs = reopen(&mut prs);
    assert_eq!(prs.chart_title(chart).unwrap().as_deref(), Some("Umsatz"));
}

#[test]
fn fills_table_and_grows_it() {
    let mut prs = open();
    let table = shape(&prs, 2, "Top Customers");
    let rows: Vec<Vec<String>> = (0..4)
        .map(|i| vec![format!("C{i}"), format!("{i}M"), format!("{i}%")])
        .collect();
    prs.fill_table(table, &rows, 1, true).unwrap();
    let prs = reopen(&mut prs);
    let values = prs.table_values(table).unwrap();
    assert_eq!(values.len(), 5);
    assert_eq!(values[0], ["Customer", "Revenue", "Share"]);
    assert_eq!(values[4], ["C3", "3M", "3%"]);
}

#[test]
fn shrinks_table_and_deletes_column() {
    let mut prs = open();
    let table = shape(&prs, 2, "Top Customers");
    prs.fill_table(table, &[vec!["Solo".into()]], 1, true)
        .unwrap();
    prs.delete_table_column(table, 2).unwrap();
    let prs = reopen(&mut prs);
    let expected: Vec<Vec<String>> = vec![
        vec!["Customer".into(), "Revenue".into()],
        vec!["Solo".into(), "1.2M".into()],
    ];
    assert_eq!(prs.table_values(table).unwrap(), expected);
}

#[test]
fn deleting_chart_shape_drops_its_parts() {
    let mut prs = open();
    let chart = shape(&prs, 3, "Trend");
    let chart_part = prs.chart_part(chart).unwrap();
    prs.delete_shape(chart).unwrap();
    let prs = reopen(&mut prs);
    assert!(
        prs.find_shape(prs.slide_at(3).unwrap(), "Trend")
            .unwrap()
            .is_none()
    );
    let parts = part_names(&prs);
    assert!(!parts.contains(&chart_part), "{chart_part} still present");
    // The other chart on the slide survives.
    assert!(prs.chart_data(shape(&prs, 3, "Share")).is_ok());
}

#[test]
fn deleting_slide_cleans_up_references() {
    let mut prs = open();
    let slide = prs.slide_at(3).unwrap();
    let part = prs.slide_part(slide).unwrap();
    prs.delete_slide(slide).unwrap();
    let mut prs = reopen(&mut prs);
    assert_eq!(prs.slide_count().unwrap(), 6);
    assert!(!part_names(&prs).contains(&part));
    let bytes = prs.to_bytes().unwrap();
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    let mut read = |name: &str| {
        use std::io::Read;
        let mut s = String::new();
        zip.by_name(name).unwrap().read_to_string(&mut s).unwrap();
        s
    };
    let pres = read("ppt/presentation.xml");
    assert!(
        !pres.contains(&format!("id=\"{}\"", slide.0)),
        "section still lists the slide"
    );
    let types = read("[Content_Types].xml");
    assert!(!types.contains(&part));
    // The slide that linked to the deleted one lost its hyperlink.
    let linking = read("ppt/slides/slide7.xml");
    assert!(!linking.contains("hlinkClick"));
    // No charts of the deleted slide remain.
    assert!(!types.contains("chart2.xml") && !types.contains("chart3.xml"));
    assert!(
        zip.by_name("ppt/embeddings/Microsoft_Excel_Sheet2.xlsx")
            .is_err()
    );
    assert!(zip.by_name("ppt/charts/chart1.xml").is_ok());
}

#[test]
fn duplicates_slide_with_independent_chart() {
    let mut prs = open();
    let original = prs.slide_at(1).unwrap();
    let copy = prs.duplicate_slide(original, None).unwrap();
    assert_eq!(prs.slide_index(copy).unwrap(), 2);
    let copy_chart = prs.find_shape(copy, "Revenue Chart").unwrap().unwrap();
    assert_ne!(
        prs.chart_part(copy_chart).unwrap(),
        prs.chart_part(shape(&prs, 1, "Revenue Chart")).unwrap()
    );
    prs.set_chart_data(
        copy_chart,
        &ChartData::new(
            Categories::Labels(vec!["X".into()]),
            vec![Series::new("S", vec![Some(1.0)])],
        ),
    )
    .unwrap();
    let prs = reopen(&mut prs);
    assert_eq!(prs.slide_count().unwrap(), 8);
    let original_data = prs.chart_data(shape(&prs, 1, "Revenue Chart")).unwrap();
    assert_eq!(original_data.series.len(), 2);
    let copy_data = prs.chart_data(shape(&prs, 2, "Revenue Chart")).unwrap();
    assert_eq!(copy_data.series.len(), 1);
}

#[test]
fn moves_slides() {
    let mut prs = open();
    let last = prs.slide_at(6).unwrap();
    prs.move_slide(last, 0).unwrap();
    let prs = reopen(&mut prs);
    assert_eq!(prs.slide_at(0).unwrap(), last);
}
