//! Bitmap renderers for report-level performance comparison graphs.

use std::{io, path::Path};

use plotters::prelude::*;

use super::CategoryMetrics;

pub(super) fn render_radar(
    output: &Path,
    categories: &[CategoryMetrics],
    old_label: &str,
    new_label: &str,
) -> io::Result<()> {
    let path = output.join("radar_chart.png");
    let path = path.to_string_lossy().into_owned();
    let root = BitMapBackend::new(&path, (1200, 700)).into_drawing_area();
    root.fill(&WHITE).map_err(plot_error)?;
    let center = (350_i32, 350_i32);
    let radius = 240.0_f64;
    for ring in 1..=5 {
        let ring_radius = radius * f64::from(ring) / 5.0;
        root.draw(&Circle::new(
            center,
            ring_radius.round() as i32,
            ShapeStyle::from(&RGBColor(210, 210, 210)),
        ))
        .map_err(plot_error)?;
    }
    let count = categories.len();
    let points = |old: bool| {
        categories
            .iter()
            .enumerate()
            .map(|(index, category)| {
                let angle = std::f64::consts::TAU * index as f64 / count as f64
                    - std::f64::consts::FRAC_PI_2;
                let value = if old {
                    category.old_capability
                } else {
                    category.new_capability
                };
                (
                    center.0 + (angle.cos() * radius * value).round() as i32,
                    center.1 + (angle.sin() * radius * value).round() as i32,
                )
            })
            .collect::<Vec<_>>()
    };
    for (index, category) in categories.iter().enumerate() {
        let angle =
            std::f64::consts::TAU * index as f64 / count as f64 - std::f64::consts::FRAC_PI_2;
        let end = (
            center.0 + (angle.cos() * radius).round() as i32,
            center.1 + (angle.sin() * radius).round() as i32,
        );
        root.draw(&PathElement::new(vec![center, end], BLACK.mix(0.25)))
            .map_err(plot_error)?;
        root.draw(&Text::new(
            category.category.clone(),
            (
                center.0 + (angle.cos() * (radius + 40.0)).round() as i32,
                center.1 + (angle.sin() * (radius + 40.0)).round() as i32,
            ),
            ("sans-serif", 22).into_font(),
        ))
        .map_err(plot_error)?;
    }
    let mut old_points = points(true);
    old_points.push(old_points[0]);
    let mut new_points = points(false);
    new_points.push(new_points[0]);
    root.draw(&Polygon::new(old_points.clone(), BLUE.mix(0.18).filled()))
        .map_err(plot_error)?;
    root.draw(&PathElement::new(old_points, BLUE.stroke_width(3)))
        .map_err(plot_error)?;
    root.draw(&Polygon::new(new_points.clone(), RED.mix(0.18).filled()))
        .map_err(plot_error)?;
    root.draw(&PathElement::new(new_points, RED.stroke_width(3)))
        .map_err(plot_error)?;
    root.draw(&Text::new(
        format!("Blue: {old_label}    Red: {new_label}"),
        (700, 50),
        ("sans-serif", 24).into_font(),
    ))
    .map_err(plot_error)?;
    for (index, category) in categories.iter().enumerate() {
        root.draw(&Text::new(
            format!(
                "{}: {:.1}% vs {:.1}% ({})",
                category.category,
                category.old_capability * 100.0,
                category.new_capability * 100.0,
                category.verdict
            ),
            (700, 100 + index as i32 * 55),
            ("sans-serif", 19).into_font(),
        ))
        .map_err(plot_error)?;
    }
    root.present().map_err(plot_error)
}

pub(super) fn render_bars(
    output: &Path,
    category: &CategoryMetrics,
    old_label: &str,
    new_label: &str,
) -> io::Result<()> {
    let name = format!(
        "{}_bar.png",
        category.category.to_ascii_lowercase().replace(' ', "_")
    );
    let path = output.join(name);
    let path = path.to_string_lossy().into_owned();
    let root = BitMapBackend::new(&path, (1000, 520)).into_drawing_area();
    root.fill(&WHITE).map_err(plot_error)?;
    let maximum = category
        .old_values
        .iter()
        .chain(&category.new_values)
        .copied()
        .fold(0.0_f64, f64::max)
        .max(1.0);
    let slots = category.labels.len() * 2;
    let mut chart = ChartBuilder::on(&root)
        .caption(
            format!("{} - {old_label} vs {new_label}", category.category),
            ("sans-serif", 24),
        )
        .margin(20)
        .x_label_area_size(70)
        .y_label_area_size(70)
        .build_cartesian_2d(0..slots, 0.0..maximum * 1.15)
        .map_err(plot_error)?;
    chart
        .configure_mesh()
        .x_labels(category.labels.len())
        .x_label_formatter(&|slot| category.labels.get(*slot / 2).cloned().unwrap_or_default())
        .draw()
        .map_err(plot_error)?;
    chart
        .draw_series(
            category
                .old_values
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    Rectangle::new([(index * 2, 0.0), (index * 2 + 1, *value)], BLUE.filled())
                }),
        )
        .map_err(plot_error)?;
    chart
        .draw_series(
            category
                .new_values
                .iter()
                .enumerate()
                .map(|(index, value)| {
                    Rectangle::new(
                        [(index * 2 + 1, 0.0), (index * 2 + 2, *value)],
                        RED.filled(),
                    )
                }),
        )
        .map_err(plot_error)?;
    root.present().map_err(plot_error)
}

fn plot_error(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}
