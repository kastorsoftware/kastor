// random device/sdk generation for accounts without .json
// data sourced from real telegram client fingerprints

use rand::Rng;

#[derive(Clone)]
pub struct DeviceInfo {
    pub device: String,
    pub sdk: String,
    pub app_version: String,
}

const ANDROID_DEVICES: &[(&str, &str)] = &[
    // samsung - sdk 29
    ("Samsung Galaxy A01", "SDK 29"), ("Samsung Galaxy A01 Core", "SDK 29"),
    ("Samsung Galaxy A11", "SDK 29"), ("Samsung Galaxy A21", "SDK 29"),
    ("Samsung Galaxy A21s", "SDK 29"), ("Samsung Galaxy A31", "SDK 29"),
    ("Samsung Galaxy A41", "SDK 29"), ("Samsung Galaxy A51", "SDK 29"),
    ("Samsung Galaxy A71", "SDK 29"), ("Samsung Galaxy A91", "SDK 29"),
    ("Samsung Galaxy F41", "SDK 29"), ("Samsung Galaxy M01", "SDK 29"),
    ("Samsung Galaxy M11", "SDK 29"), ("Samsung Galaxy M21", "SDK 29"),
    ("Samsung Galaxy M31", "SDK 29"), ("Samsung Galaxy M31s", "SDK 29"),
    ("Samsung Galaxy M51", "SDK 29"), ("Samsung Galaxy Note10 Lite", "SDK 29"),
    ("Samsung Galaxy S10 Lite", "SDK 29"), ("Samsung Galaxy S20", "SDK 29"),
    ("Samsung Galaxy S20+", "SDK 29"), ("Samsung Galaxy S20 Ultra", "SDK 29"),
    ("Samsung Galaxy S20 FE", "SDK 29"),
    // samsung - sdk 30
    ("Samsung Galaxy A02", "SDK 30"), ("Samsung Galaxy A02s", "SDK 30"),
    ("Samsung Galaxy A12", "SDK 30"), ("Samsung Galaxy A22", "SDK 30"),
    ("Samsung Galaxy A32", "SDK 30"), ("Samsung Galaxy A32 5G", "SDK 30"),
    ("Samsung Galaxy A42 5G", "SDK 30"), ("Samsung Galaxy A51 5G", "SDK 30"),
    ("Samsung Galaxy A52", "SDK 30"), ("Samsung Galaxy A52 5G", "SDK 30"),
    ("Samsung Galaxy A71 5G", "SDK 30"), ("Samsung Galaxy A72", "SDK 30"),
    ("Samsung Galaxy F12", "SDK 30"), ("Samsung Galaxy F22", "SDK 30"),
    ("Samsung Galaxy F62", "SDK 30"), ("Samsung Galaxy M02s", "SDK 30"),
    ("Samsung Galaxy M12", "SDK 30"), ("Samsung Galaxy M22", "SDK 30"),
    ("Samsung Galaxy M32", "SDK 30"), ("Samsung Galaxy M42 5G", "SDK 30"),
    ("Samsung Galaxy M52 5G", "SDK 30"), ("Samsung Galaxy M62", "SDK 30"),
    ("Samsung Galaxy Note20", "SDK 30"), ("Samsung Galaxy Note20 Ultra", "SDK 30"),
    ("Samsung Galaxy S21", "SDK 30"), ("Samsung Galaxy S21 5G", "SDK 30"),
    ("Samsung Galaxy S21 Ultra 5G", "SDK 30"), ("Samsung Galaxy Tab A7 10.4", "SDK 30"),
    ("Samsung Galaxy Z Flip3 5G", "SDK 30"), ("Samsung Galaxy Z Fold3 5G", "SDK 30"),
    // samsung - sdk 31
    ("Samsung Galaxy A03", "SDK 31"), ("Samsung Galaxy A03s", "SDK 31"),
    ("Samsung Galaxy A13 5G", "SDK 31"), ("Samsung Galaxy A23", "SDK 31"),
    ("Samsung Galaxy A33 5G", "SDK 31"), ("Samsung Galaxy A52s 5G", "SDK 31"),
    ("Samsung Galaxy A53 5G", "SDK 31"), ("Samsung Galaxy A73 5G", "SDK 31"),
    ("Samsung Galaxy S22 5G", "SDK 31"), ("Samsung Galaxy Z Flip4", "SDK 31"),
    // samsung - sdk 33
    ("Samsung Galaxy A05", "SDK 33"), ("Samsung Galaxy A14", "SDK 33"),
    ("Samsung Galaxy A14 5G", "SDK 33"), ("Samsung Galaxy A34", "SDK 33"),
    ("Samsung Galaxy A54", "SDK 33"), ("Samsung Galaxy F14", "SDK 33"),
    ("Samsung Galaxy F34", "SDK 33"), ("Samsung Galaxy M14", "SDK 33"),
    ("Samsung Galaxy M34 5G", "SDK 33"), ("Samsung Galaxy M54", "SDK 33"),
    ("Samsung Galaxy S23", "SDK 33"), ("Samsung Galaxy S23 Ultra", "SDK 33"),
    ("Samsung Galaxy S23+", "SDK 33"), ("Samsung Galaxy Z Flip5", "SDK 33"),
    ("Samsung Galaxy Z Fold5", "SDK 33"),
    // xiaomi - sdk 29
    ("Xiaomi Redmi 9", "SDK 29"), ("Xiaomi Redmi 9A", "SDK 29"),
    ("Xiaomi Redmi 9C", "SDK 29"), ("Xiaomi Redmi Note 9", "SDK 29"),
    ("Xiaomi Redmi Note 9 Pro", "SDK 29"), ("Xiaomi Redmi Note 9S", "SDK 29"),
    ("Xiaomi Redmi K30", "SDK 29"), ("Xiaomi Mi 10 5G", "SDK 29"),
    ("Xiaomi Mi 10 Pro 5G", "SDK 29"), ("Xiaomi Poco X2", "SDK 29"),
    ("Xiaomi Poco C3", "SDK 29"), ("Xiaomi Poco F2 Pro", "SDK 29"),
    ("Xiaomi Black Shark 3", "SDK 29"),
    // xiaomi - sdk 30
    ("Xiaomi Redmi 9T", "SDK 30"), ("Xiaomi Redmi 10", "SDK 30"),
    ("Xiaomi Redmi 10A", "SDK 30"), ("Xiaomi Redmi 10C", "SDK 30"),
    ("Xiaomi Redmi Note 10", "SDK 30"), ("Xiaomi Redmi Note 10 Pro", "SDK 30"),
    ("Xiaomi Redmi Note 10S", "SDK 30"), ("Xiaomi Redmi Note 11", "SDK 30"),
    ("Xiaomi Redmi Note 11 Pro", "SDK 30"), ("Xiaomi Redmi Note 11S", "SDK 30"),
    ("Xiaomi Redmi K40", "SDK 30"), ("Xiaomi Redmi K40 Pro", "SDK 30"),
    ("Xiaomi Mi 10T 5G", "SDK 30"), ("Xiaomi Mi 11", "SDK 30"),
    ("Xiaomi Mi 11 Lite", "SDK 30"), ("Xiaomi Mi 11 Pro", "SDK 30"),
    ("Xiaomi Mi 11 Ultra", "SDK 30"), ("Xiaomi Poco X3", "SDK 30"),
    ("Xiaomi Poco X3 Pro", "SDK 30"), ("Xiaomi Poco M3", "SDK 30"),
    ("Xiaomi Poco F3", "SDK 30"), ("Xiaomi Black Shark 4", "SDK 30"),
    // xiaomi - sdk 31
    ("Xiaomi Redmi Note 12", "SDK 31"), ("Xiaomi Redmi Note 12 Pro", "SDK 31"),
    ("Xiaomi Redmi K50", "SDK 31"), ("Xiaomi 12", "SDK 31"),
    ("Xiaomi 12 Pro", "SDK 31"), ("Xiaomi Poco X4 Pro 5G", "SDK 31"),
    ("Xiaomi Poco M4 Pro", "SDK 31"), ("Xiaomi Poco F4", "SDK 31"),
    // xiaomi - sdk 33
    ("Xiaomi 13", "SDK 33"), ("Xiaomi 13 Pro", "SDK 33"),
    ("Xiaomi 13 Ultra", "SDK 33"), ("Xiaomi 13T", "SDK 33"),
    ("Xiaomi 13T Pro", "SDK 33"), ("Xiaomi Poco F5", "SDK 33"),
    ("Xiaomi Poco X6", "SDK 33"), ("Xiaomi Redmi 12", "SDK 33"),
    ("Xiaomi Redmi 13C", "SDK 33"), ("Xiaomi Redmi K60 Pro", "SDK 33"),
    ("Xiaomi Redmi Note 13", "SDK 33"), ("Xiaomi Redmi Note 13 Pro", "SDK 33"),
    ("Xiaomi Redmi Note 13 Pro+", "SDK 33"),
    // huawei
    ("Huawei Enjoy 10e", "SDK 29"), ("Huawei Enjoy 20 5G", "SDK 30"),
    ("Huawei Mate 30", "SDK 29"), ("Huawei Mate 30 Pro", "SDK 29"),
    ("Huawei Mate 40", "SDK 30"), ("Huawei Mate 40 Pro", "SDK 30"),
    ("Huawei P40", "SDK 29"), ("Huawei P40 Pro", "SDK 29"),
    ("Huawei P40 lite", "SDK 29"), ("Huawei nova 6", "SDK 29"),
    ("Huawei nova 7 5G", "SDK 30"), ("Huawei nova 8 5G", "SDK 30"),
    ("Huawei nova 8 Pro 5G", "SDK 30"), ("Huawei nova 8 SE", "SDK 30"),
    ("Huawei nova Y60", "SDK 30"), ("Huawei nova Y90", "SDK 30"),
    ("Huawei MatePad 10.4", "SDK 29"), ("Huawei MatePad Pro 10.8", "SDK 29"),
    ("Huawei Mate 50 Pro", "SDK 33"), ("Huawei Mate Xs 2", "SDK 30"),
    // motorola
    ("Motorola Moto G Power", "SDK 29"), ("Motorola Moto G Stylus", "SDK 29"),
    ("Motorola Moto G9 Play", "SDK 29"), ("Motorola Moto G9 Plus", "SDK 29"),
    ("Motorola Moto G10", "SDK 30"), ("Motorola Moto G20", "SDK 30"),
    ("Motorola Moto G30", "SDK 30"), ("Motorola Moto G50", "SDK 30"),
    ("Motorola Moto G60", "SDK 30"), ("Motorola Moto G71 5G", "SDK 30"),
    ("Motorola Moto G82", "SDK 31"), ("Motorola Moto G22", "SDK 31"),
    ("Motorola Moto G32", "SDK 31"), ("Motorola Moto G42", "SDK 31"),
    ("Motorola Moto G52", "SDK 31"), ("Motorola Moto G72", "SDK 31"),
    ("Motorola Edge", "SDK 29"), ("Motorola Edge 20", "SDK 30"),
    ("Motorola Edge 30", "SDK 31"), ("Motorola Edge 30 Pro", "SDK 31"),
    ("Motorola Edge 40", "SDK 33"), ("Motorola Razr 5G", "SDK 30"),
    // realme
    ("Realme 6", "SDK 29"), ("Realme 7", "SDK 29"),
    ("Realme 7 Pro", "SDK 29"), ("Realme 8", "SDK 30"),
    ("Realme 8 Pro", "SDK 30"), ("Realme 9", "SDK 31"),
    ("Realme 9 Pro", "SDK 31"), ("Realme 10", "SDK 31"),
    ("Realme 11 Pro", "SDK 33"), ("Realme C11", "SDK 29"),
    ("Realme C15", "SDK 29"), ("Realme C21", "SDK 30"),
    ("Realme C25", "SDK 30"), ("Realme C30", "SDK 30"),
    ("Realme C31", "SDK 30"), ("Realme C35", "SDK 30"),
    ("Realme Narzo 20", "SDK 29"), ("Realme Narzo 30", "SDK 30"),
    ("Realme Narzo 50", "SDK 31"), ("Realme GT 5G", "SDK 30"),
    ("Realme GT Neo", "SDK 30"), ("Realme GT Neo2", "SDK 30"),
    ("Realme GT2 Pro", "SDK 31"), ("Realme Pad", "SDK 30"),
    ("Realme GT3", "SDK 33"), ("Realme C53", "SDK 33"),
    ("Realme Narzo 60", "SDK 33"),
];

const DESKTOP_DEVICES: &[(&str, &str)] = &[
    // asus laptops
    ("GA401QE-K2165TS", "Windows 10 x64"), ("GA402RJZ-L4135WS", "Windows 11 x64"),
    ("GA401IHR-K2066TS", "Windows 10 x64"), ("GA503RM-LN095WS", "Windows 11 x64"),
    ("FX516PCZ-HN091T", "Windows 10 x64"), ("GU604VZ-NM050WS", "Windows 11 x64"),
    ("TUF516PE-AB73", "Windows 10 x64"), ("FA506QM-HN008TS", "Windows 10 x64"),
    ("FA706ICB-HX061W", "Windows 11 x64"), ("FX506LH-HN258T", "Windows 10 x64"),
    ("FX566LH-HN009T", "Windows 10 x64"), ("FX766LI-H7059TS", "Windows 10 x64"),
    ("KM513UA-L503TS", "Windows 11 x64"), ("M6500QF-HN741WS", "Windows 11 x64"),
    ("K413EA-EB311WS", "Windows 11 x64"), ("UX482EG-KA711TS", "Windows 11 x64"),
    ("G513QM-HF313TS", "Windows 10 x64"), ("G713QM-K4215TS", "Windows 10 x64"),
    ("UM5302TA-LX701WS", "Windows 11 x64"), ("GA401II-HE169TS", "Windows 10 x64"),
    ("G512LI-HN081T", "Windows 10 x64"), ("GZ301ZA-LD049WS", "Windows 11 x64"),
    ("UX482EGR-KA521WS", "Windows 11 x64"), ("GA401QC-HZ063TS", "Windows 10 x64"),
    ("GX701LXS-HG002TS", "Windows 10 x64"), ("GA401IU-HA246TS", "Windows 10 x64"),
    ("GA503QR-HQ133TS", "Windows 10 x64"), ("G713RM-LL167WS", "Windows 11 x64"),
    ("FX516PM-AZ155TS", "Windows 10 x64"), ("UX362FA-EL501T", "Windows 10 x64"),
    ("G513RM-HQ038WS", "Windows 11 x64"), ("M1405YA-KM541WS", "Windows 11 x64"),
    ("FA506II-AL117T", "Windows 10 x64"), ("S5504VA-MA953WS", "Windows 11 x64"),
    ("GA402XV-N2033WS", "Windows 11 x64"), ("UM5401QA-KM541WS", "Windows 11 x64"),
    ("GU603ZX-K8024WS", "Windows 11 x64"), ("GV301RE-LI201WS", "Windows 11 x64"),
    ("E510MA-EJ001W", "Windows 11 x64"), ("X513EA-BQ312TS", "Windows 10 x64"),
    // hp laptops
    ("x360-14-dh1008tu", "Windows 10 x64"), ("15-dk", "Windows 10 x64"),
    ("15-ec1000", "Windows 10 x64"), ("15-ec1073dx", "Windows 10 x64"),
    ("15-ds1010wm", "Windows 11 x64"), ("15-ds1063cl", "Windows 11 x64"),
    ("8MO-075IN", "Windows 11 x64"),
    // lenovo laptops
    ("X360", "Windows 11 x64"), ("15ACH6", "Windows 11 x64"),
    ("XL25", "Windows 11 x64"), ("15ITL05", "Windows 11 x64"),
    ("16ACH6H", "Windows 11 x64"), ("15IML05", "Windows 11 x64"),
    ("16ARH7H", "Windows 11 x64"),
    // samsung laptops
    ("NP940X3M", "Windows 10 x64"), ("NP750XFG-KB1IN", "Windows 11 x64"),
    ("XE530QDA-KA2US", "Windows 11 x64"),
    // huawei laptops
    ("Mach-W19B", "Windows 10 x64"),
    // msi laptops
    ("A11M-436IN", "Windows 11 x64"), ("B5EEK-069IN", "Windows 11 x64"),
    // generic desktops
    ("Satellite A215", "Windows 10 x64"), ("Latitude 7490", "Windows 10 x64"),
    ("ThinkPad X1 Carbon", "Windows 10 x64"), ("Pavilion 15", "Windows 10 x64"),
    ("IdeaPad 5", "Windows 10 x64"), ("TP200SA-DH01T", "Windows 10 x64"),
    ("K3504VA-LK552WS", "Windows 11 x64"), ("E1504FA-NJ322WS", "Windows 11 x64"),
    ("FA577RM-HQ032WS", "Windows 11 x64"), ("K6602HC-N1901WS", "Windows 11 x64"),
    ("GV302XU-MU013WS", "Windows 11 x64"), ("TP470EA-EC301TS", "Windows 10 x64"),
    ("GU603ZM-K8035WS", "Windows 11 x64"), ("G513RM-HF194WS", "Windows 11 x64"),
    ("GA401QH-BM072TS", "Windows 10 x64"), ("FX516PMZ-HN186WS", "Windows 11 x64"),
    ("GA402XZ-N2020WS", "Windows 11 x64"), ("FA566IU-HN249T", "Windows 10 x64"),
    ("FA506IC-HN075W", "Windows 11 x64"), ("G614JU-N3201WS", "Windows 11 x64"),
    ("FA566QM-HN087TS", "Windows 10 x64"), ("UX3402VA-KM541WS", "Windows 11 x64"),
    ("G814JI-N6097WS", "Windows 11 x64"), ("GA503RMZ-HQ153WS", "Windows 11 x64"),
    ("FX506HM-HN016T", "Windows 10 x64"), ("FX506HF-HN025W", "Windows 11 x64"),
    ("G713PI-LL057WS", "Windows 11 x64"), ("G713PU-LL060WS", "Windows 11 x64"),
    ("FA777XU-HX026WS", "Windows 11 x64"), ("FX577ZE-HN072WS", "Windows 11 x64"),
];

const DESKTOP_APP_VERSIONS: &[&str] = &[
    "5.5.5 x64", "5.8.3 x64", "5.12.1 x64",
    "6.0.2 x64", "6.2.4 x64", "6.4.2 x64",
    "6.5.6 x64", "6.7.1 x64", "6.8.0 x64",
    "6.8.2 x64", "6.8.1 x64",
];

const ANDROID_APP_VERSIONS: &[&str] = &[
    "9.6.7", "9.7.6", "10.0.9", "10.2.4", "10.3.2",
    "10.6.2", "10.8.1", "10.9.1", "10.12.0", "10.14.5",
];

pub fn generate_random_device() -> DeviceInfo {
    let mut rng = rand::thread_rng();

    // 60% desktop, 40% android
    if rng.gen_bool(0.6) {
        let (device, sdk) = DESKTOP_DEVICES[rng.gen_range(0..DESKTOP_DEVICES.len())];
        let app_ver = DESKTOP_APP_VERSIONS[rng.gen_range(0..DESKTOP_APP_VERSIONS.len())];
        DeviceInfo {
            device: device.to_string(),
            sdk: sdk.to_string(),
            app_version: app_ver.to_string(),
        }
    } else {
        let (device, sdk) = ANDROID_DEVICES[rng.gen_range(0..ANDROID_DEVICES.len())];
        let app_ver = ANDROID_APP_VERSIONS[rng.gen_range(0..ANDROID_APP_VERSIONS.len())];
        DeviceInfo {
            device: device.to_string(),
            sdk: sdk.to_string(),
            app_version: app_ver.to_string(),
        }
    }
}
