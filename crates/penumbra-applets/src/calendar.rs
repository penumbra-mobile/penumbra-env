use chrono::{Datelike, Duration, NaiveDate};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DayCell {
    pub day: u32,
    pub in_month: bool,
    pub is_today: bool,
}

/// Generates a 42-cell (6 weeks x 7 days, Monday-first) calendar grid for `(year, month)`.
pub fn month_grid(year: i32, month: u32, today: NaiveDate) -> Vec<DayCell> {
    let first = NaiveDate::from_ymd_opt(year, month, 1).unwrap_or(today);
    let offset = first.weekday().num_days_from_monday() as i64;
    let start = first - Duration::days(offset);

    (0..42)
        .map(|i| {
            let d = start + Duration::days(i);
            DayCell {
                day: d.day(),
                in_month: d.month() == month && d.year() == year,
                is_today: d == today,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_month_grid_has_42_cells_and_marks_today() {
        let today = NaiveDate::from_ymd_opt(2026, 11, 18).unwrap();
        let grid = month_grid(2026, 11, today);
        assert_eq!(grid.len(), 42);
        assert_eq!(grid.iter().filter(|c| c.is_today).count(), 1);
        assert!(grid.iter().any(|c| c.day == 18 && c.in_month && c.is_today));
    }
}

