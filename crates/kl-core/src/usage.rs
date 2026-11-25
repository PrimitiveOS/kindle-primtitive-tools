//! Battery use, as the owner sees it: how long the screen was on, how often it woke, and each app's time in front and
//! CPU time. The Kindle has no per-app power meter, so an app's share is an estimate built from those two figures. Days
//! are kept for `KEEP_DAYS`; the log stays on the device.

use std::collections::BTreeMap;

/// How many days of usage are kept.
pub const KEEP_DAYS: u32 = 30;
/// The part of an app's estimate that comes from CPU time; the rest comes from time in front.
const CPU_WEIGHT: f64 = 0.5;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AppUse {
    pub front_secs: u64,
    pub cpu_ticks: u64,
}

/// One day: days since 1970-01-01.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Day {
    pub day: u32,
    pub screen_on_secs: u64,
    pub wakes: u32,
    pub apps: BTreeMap<String, AppUse>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UsageLog {
    pub days: Vec<Day>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppShare {
    pub app: String,
    pub front_secs: u64,
    pub cpu_ticks: u64,
    /// The estimated share of battery use, in percent (not a measurement).
    pub share_percent: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Summary {
    pub screen_on_secs: u64,
    pub wakes: u32,
    /// Apps, most used first.
    pub apps: Vec<AppShare>,
}

/// The day number (days since 1970-01-01) of a Unix time in seconds.
pub fn day_of(unix_secs: u64) -> u32 {
    (unix_secs / 86_400) as u32
}

impl UsageLog {
    fn day_mut(&mut self, day: u32) -> &mut Day {
        let at = match self.days.iter().position(|d| d.day == day) {
            Some(at) => at,
            None => {
                self.days.push(Day { day, ..Day::default() });
                self.days.sort_by_key(|d| d.day);
                self.days.iter().position(|d| d.day == day).unwrap_or(0)
            }
        };
        &mut self.days[at]
    }

    pub fn add_screen_on(&mut self, day: u32, secs: u64) {
        self.day_mut(day).screen_on_secs += secs;
    }

    pub fn add_wake(&mut self, day: u32) {
        self.day_mut(day).wakes += 1;
    }

    pub fn add_app(&mut self, day: u32, app: &str, front_secs: u64, cpu_ticks: u64) {
        let entry = self.day_mut(day).apps.entry(app.to_string()).or_default();
        entry.front_secs += front_secs;
        entry.cpu_ticks += cpu_ticks;
    }

    /// Drop the days older than `KEEP_DAYS` before `today`, and any day after it (a clock that went back).
    pub fn prune(&mut self, today: u32) {
        self.days.retain(|d| d.day <= today && d.day + KEEP_DAYS > today);
    }

    /// Totals for the days from `since` (inclusive) on, with each app's estimated share.
    pub fn summarize(&self, since: u32) -> Summary {
        let mut screen_on_secs = 0;
        let mut wakes = 0;
        let mut apps: BTreeMap<&str, AppUse> = BTreeMap::new();
        for day in self.days.iter().filter(|d| d.day >= since) {
            screen_on_secs += day.screen_on_secs;
            wakes += day.wakes;
            for (name, use_) in &day.apps {
                let total = apps.entry(name.as_str()).or_default();
                total.front_secs += use_.front_secs;
                total.cpu_ticks += use_.cpu_ticks;
            }
        }
        let front_total: u64 = apps.values().map(|a| a.front_secs).sum();
        let cpu_total: u64 = apps.values().map(|a| a.cpu_ticks).sum();
        let share = |part: u64, whole: u64| if whole == 0 { 0.0 } else { part as f64 / whole as f64 };
        let mut shares: Vec<AppShare> = apps
            .iter()
            .map(|(name, use_)| {
                let score = CPU_WEIGHT * share(use_.cpu_ticks, cpu_total) + (1.0 - CPU_WEIGHT) * share(use_.front_secs, front_total);
                AppShare {
                    app: name.to_string(),
                    front_secs: use_.front_secs,
                    cpu_ticks: use_.cpu_ticks,
                    share_percent: (score * 100.0).round() as u32,
                }
            })
            .collect();
        shares.sort_by(|a, b| b.share_percent.cmp(&a.share_percent).then_with(|| a.app.cmp(&b.app)));
        Summary { screen_on_secs, wakes, apps: shares }
    }

    /// The saved form: a line per day (`D day screen_on wakes`) and per app and day (`A day app front cpu`).
    pub fn to_text(&self) -> String {
        let mut out = String::new();
        for day in &self.days {
            out.push_str(&format!("D\t{}\t{}\t{}\n", day.day, day.screen_on_secs, day.wakes));
            for (app, use_) in &day.apps {
                out.push_str(&format!("A\t{}\t{}\t{}\t{}\n", day.day, app, use_.front_secs, use_.cpu_ticks));
            }
        }
        out
    }

    /// Read the saved form. Lines that do not parse are skipped, so a damaged file keeps what it can.
    pub fn parse(text: &str) -> UsageLog {
        let mut log = UsageLog::default();
        for line in text.lines() {
            let fields: Vec<&str> = line.split('\t').collect();
            match fields.as_slice() {
                ["D", day, screen, wakes] => {
                    if let (Ok(day), Ok(screen), Ok(wakes)) = (day.parse(), screen.parse(), wakes.parse()) {
                        let entry = log.day_mut(day);
                        entry.screen_on_secs = screen;
                        entry.wakes = wakes;
                    }
                }
                ["A", day, app, front, cpu] if !app.is_empty() => {
                    if let (Ok(day), Ok(front), Ok(cpu)) = (day.parse(), front.parse(), cpu.parse()) {
                        log.day_mut(day).apps.insert(app.to_string(), AppUse { front_secs: front, cpu_ticks: cpu });
                    }
                }
                _ => {}
            }
        }
        log
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn totals_add_up_over_the_days_in_the_window() {
        let mut log = UsageLog::default();
        log.add_screen_on(10, 600);
        log.add_wake(10);
        log.add_screen_on(11, 300);
        log.add_wake(11);
        log.add_wake(11);
        let s = log.summarize(10);
        assert_eq!((s.screen_on_secs, s.wakes), (900, 3));
        let later = log.summarize(11);
        assert_eq!((later.screen_on_secs, later.wakes), (300, 2), "a window starts at its first day");
    }

    #[test]
    fn an_app_share_follows_its_cpu_and_its_time_in_front() {
        let mut log = UsageLog::default();
        log.add_app(5, "snake", 600, 100);
        log.add_app(5, "notes", 600, 300);
        let s = log.summarize(0);
        assert_eq!(s.apps[0].app, "notes", "the most used app comes first");
        // notes: half of the CPU (300 of 400) and half of the time in front (600 of 1200): 0.5*0.75 + 0.5*0.5 = 62.5 %.
        assert_eq!(s.apps[0].share_percent, 63);
        assert_eq!(s.apps[1].share_percent, 38);
    }

    #[test]
    fn no_use_gives_no_shares() {
        let log = UsageLog::default();
        let s = log.summarize(0);
        assert!(s.apps.is_empty());
        assert_eq!((s.screen_on_secs, s.wakes), (0, 0));
    }

    #[test]
    fn only_the_last_thirty_days_are_kept() {
        let mut log = UsageLog::default();
        log.add_wake(100);
        log.add_wake(129);
        log.add_wake(130);
        log.add_wake(140);
        log.prune(130);
        let days: Vec<u32> = log.days.iter().map(|d| d.day).collect();
        assert_eq!(days, vec![129, 130], "day 100 is 30 days old and goes");
        assert!(!days.contains(&140), "a day after today is dropped");
    }

    #[test]
    fn the_saved_form_reads_back_and_skips_damage() {
        let mut log = UsageLog::default();
        log.add_screen_on(20, 90);
        log.add_wake(20);
        log.add_app(20, "tasks", 45, 7);
        let text = log.to_text() + "garbage line\nA\t20\t\t1\t1\nD\tnot-a-day\t1\t1\n";
        assert_eq!(UsageLog::parse(&text), log);
    }

    #[test]
    fn a_unix_time_gives_its_day() {
        assert_eq!(day_of(0), 0);
        assert_eq!(day_of(86_400 * 20 + 3_600), 20);
    }
}
