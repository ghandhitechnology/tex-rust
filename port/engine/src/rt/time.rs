//! C `time`, `gmtime`, `localtime` and `gettimeofday` equivalents.
//! Local time uses the Windows time-zone rules through kernel32 (an OS call, not a C
//! library), like the C runtime's localtime does when TZ is unset.

#[derive(Clone, Copy, Debug, Default)]
pub struct Tm {
    pub sec: i32,
    pub min: i32,
    pub hour: i32,
    pub mday: i32,
    pub mon: i32,  // 0..11
    pub year: i32, // years since 1900
    pub yday: i32, // 0..365
    pub wday: i32,
}

pub fn now_unix() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

pub fn now_secs_micros() -> (i64, i32) {
    let d = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    (d.as_secs() as i64, d.subsec_micros() as i32)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn is_leap(y: i64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

pub fn gmtime(t: i64) -> Tm {
    let days = t.div_euclid(86400);
    let rem = t.rem_euclid(86400);
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    let yday = days - days_from_civil(y, 1, 1);
    Tm {
        sec: (rem % 60) as i32,
        min: ((rem / 60) % 60) as i32,
        hour: (rem / 3600) as i32,
        mday: d as i32,
        mon: (m - 1) as i32,
        year: (y - 1900) as i32,
        yday: yday as i32,
        wday: ((days + 4).rem_euclid(7)) as i32,
    }
}

#[repr(C)]
#[derive(Default, Clone, Copy)]
struct SystemTime {
    w_year: u16,
    w_month: u16,
    w_day_of_week: u16,
    w_day: u16,
    w_hour: u16,
    w_minute: u16,
    w_second: u16,
    w_milliseconds: u16,
}

#[link(name = "kernel32")]
extern "system" {
    fn SystemTimeToTzSpecificLocalTime(tz: *const u8, utc: *const SystemTime, local: *mut SystemTime) -> i32;
}

pub fn localtime(t: i64) -> Tm {
    let g = gmtime(t);
    let utc = SystemTime {
        w_year: (g.year + 1900) as u16,
        w_month: (g.mon + 1) as u16,
        w_day_of_week: g.wday as u16,
        w_day: g.mday as u16,
        w_hour: g.hour as u16,
        w_minute: g.min as u16,
        w_second: g.sec as u16,
        w_milliseconds: 0,
    };
    let mut local = SystemTime::default();
    let ok = unsafe { SystemTimeToTzSpecificLocalTime(std::ptr::null(), &utc, &mut local) };
    if ok == 0 {
        return g;
    }
    let y = local.w_year as i64;
    let days = days_from_civil(y, local.w_month as i64, local.w_day as i64);
    Tm {
        sec: local.w_second as i32,
        min: local.w_minute as i32,
        hour: local.w_hour as i32,
        mday: local.w_day as i32,
        mon: local.w_month as i32 - 1,
        year: (y - 1900) as i32,
        yday: (days - days_from_civil(y, 1, 1)) as i32,
        wday: ((days + 4).rem_euclid(7)) as i32,
    }
}

#[allow(dead_code)]
pub fn days_in_year(y: i64) -> i64 {
    if is_leap(y) {
        366
    } else {
        365
    }
}
