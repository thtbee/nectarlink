// SPDX-License-Identifier: GPL-3.0-or-later
//! What the lock screen tile says, in the languages NectarLink speaks. LogonUI
//! can't read the app's translation files, so the few strings live here.

/// One of the tile's messages.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Text {
    /// The tile's title.
    Title,
    /// Under the title, before anything happens.
    Hint,
    /// The button.
    Unlock,
    /// The phone's owner said no, or didn't answer in time.
    Declined,
    /// This user isn't set up (or must set it up again).
    NotEnrolled,
    /// The phone isn't connected.
    PhoneAway,
    /// Too many failed tries.
    LockedOut,
    /// NectarLink isn't running in this session.
    NotRunning,
    /// Windows refused the password it got.
    Rejected,
    /// Anything else.
    Failed,
}

/// The language to show: the primary language ID Windows reports.
pub fn get(text: Text, primary_language: u16) -> &'static str {
    let i = text as usize;
    let table: &[&str; 10] = match primary_language {
        0x07 => &DE,
        0x0a => &ES,
        0x11 => &JA,
        0x15 => &PL,
        0x16 => &PT,
        0x19 => &RU,
        0x1f => &TR,
        0x2a => &VI,
        0x04 => &ZH,
        _ => &EN,
    };
    table[i]
}

const EN: [&str; 10] = [
    "Unlock with phone",
    "Confirm with your fingerprint on your phone.",
    "Unlock",
    "Not confirmed on the phone.",
    "This account isn't set up to unlock with a phone. Turn it on in NectarLink on this PC.",
    "Your phone isn't connected. Bring it close, on the same network.",
    "Too many tries. Use your password, or try again later.",
    "NectarLink isn't running for this account.",
    "Windows didn't accept the password. If you changed it, set up unlocking again in NectarLink.",
    "Couldn't unlock with the phone. Use your password.",
];
const DE: [&str; 10] = [
    "Mit Telefon entsperren",
    "Bestätige mit deinem Fingerabdruck am Telefon.",
    "Entsperren",
    "Am Telefon nicht bestätigt.",
    "Dieses Konto ist nicht zum Entsperren mit dem Telefon eingerichtet. Schalte es in NectarLink auf diesem PC ein.",
    "Dein Telefon ist nicht verbunden. Bring es in die Nähe, im selben Netzwerk.",
    "Zu viele Versuche. Nutze dein Kennwort oder versuche es später.",
    "NectarLink läuft für dieses Konto nicht.",
    "Windows hat das Kennwort nicht angenommen. Wenn du es geändert hast, richte das Entsperren in NectarLink neu ein.",
    "Entsperren mit dem Telefon hat nicht geklappt. Nutze dein Kennwort.",
];
const ES: [&str; 10] = [
    "Desbloquear con el teléfono",
    "Confirma con tu huella en el teléfono.",
    "Desbloquear",
    "No se confirmó en el teléfono.",
    "Esta cuenta no está configurada para desbloquearse con el teléfono. Actívalo en NectarLink en este PC.",
    "Tu teléfono no está conectado. Acércalo, en la misma red.",
    "Demasiados intentos. Usa tu contraseña o inténtalo más tarde.",
    "NectarLink no se está ejecutando para esta cuenta.",
    "Windows no aceptó la contraseña. Si la cambiaste, vuelve a configurar el desbloqueo en NectarLink.",
    "No se pudo desbloquear con el teléfono. Usa tu contraseña.",
];
const JA: [&str; 10] = [
    "スマートフォンでロック解除",
    "スマートフォンで指紋を使って確認してください。",
    "ロック解除",
    "スマートフォンで確認されませんでした。",
    "このアカウントはスマートフォンでのロック解除が設定されていません。この PC の NectarLink でオンにしてください。",
    "スマートフォンが接続されていません。同じネットワークで近くに置いてください。",
    "試行回数が多すぎます。パスワードを使うか、後でもう一度お試しください。",
    "このアカウントで NectarLink が実行されていません。",
    "Windows がパスワードを受け付けませんでした。変更した場合は NectarLink でロック解除を設定し直してください。",
    "スマートフォンでロック解除できませんでした。パスワードを使ってください。",
];
const PL: [&str; 10] = [
    "Odblokuj telefonem",
    "Potwierdź odciskiem palca na telefonie.",
    "Odblokuj",
    "Nie potwierdzono na telefonie.",
    "To konto nie ma włączonego odblokowywania telefonem. Włącz je w NectarLink na tym komputerze.",
    "Telefon nie jest połączony. Przybliż go, w tej samej sieci.",
    "Zbyt wiele prób. Użyj hasła albo spróbuj później.",
    "NectarLink nie działa dla tego konta.",
    "Windows nie przyjął hasła. Jeśli je zmieniłeś, skonfiguruj odblokowywanie w NectarLink od nowa.",
    "Nie udało się odblokować telefonem. Użyj hasła.",
];
const PT: [&str; 10] = [
    "Desbloquear com o celular",
    "Confirme com sua digital no celular.",
    "Desbloquear",
    "Não confirmado no celular.",
    "Esta conta não está configurada para desbloquear com o celular. Ative no NectarLink deste PC.",
    "Seu celular não está conectado. Deixe-o perto, na mesma rede.",
    "Tentativas demais. Use sua senha ou tente mais tarde.",
    "O NectarLink não está em execução para esta conta.",
    "O Windows não aceitou a senha. Se você a mudou, configure o desbloqueio de novo no NectarLink.",
    "Não foi possível desbloquear com o celular. Use sua senha.",
];
const RU: [&str; 10] = [
    "Разблокировать телефоном",
    "Подтвердите отпечатком пальца на телефоне.",
    "Разблокировать",
    "Не подтверждено на телефоне.",
    "Для этой учётной записи не настроена разблокировка телефоном. Включите её в NectarLink на этом ПК.",
    "Телефон не подключён. Поднесите его ближе, в той же сети.",
    "Слишком много попыток. Используйте пароль или попробуйте позже.",
    "NectarLink не запущен для этой учётной записи.",
    "Windows не принял пароль. Если вы его меняли, настройте разблокировку в NectarLink заново.",
    "Не удалось разблокировать телефоном. Используйте пароль.",
];
const TR: [&str; 10] = [
    "Telefonla kilidi aç",
    "Telefonunda parmak izinle onayla.",
    "Kilidi aç",
    "Telefonda onaylanmadı.",
    "Bu hesap telefonla kilit açmaya ayarlı değil. Bu bilgisayardaki NectarLink'te aç.",
    "Telefonun bağlı değil. Aynı ağda yakına getir.",
    "Çok fazla deneme. Parolanı kullan ya da daha sonra dene.",
    "NectarLink bu hesap için çalışmıyor.",
    "Windows parolayı kabul etmedi. Değiştirdiysen NectarLink'te kilit açmayı yeniden ayarla.",
    "Telefonla kilit açılamadı. Parolanı kullan.",
];
const VI: [&str; 10] = [
    "Mở khóa bằng điện thoại",
    "Xác nhận bằng vân tay trên điện thoại.",
    "Mở khóa",
    "Chưa được xác nhận trên điện thoại.",
    "Tài khoản này chưa được thiết lập để mở khóa bằng điện thoại. Bật trong NectarLink trên PC này.",
    "Điện thoại chưa kết nối. Để gần, cùng mạng.",
    "Thử quá nhiều lần. Dùng mật khẩu hoặc thử lại sau.",
    "NectarLink không chạy cho tài khoản này.",
    "Windows không chấp nhận mật khẩu. Nếu bạn đã đổi, hãy thiết lập lại mở khóa trong NectarLink.",
    "Không mở khóa được bằng điện thoại. Hãy dùng mật khẩu.",
];
const ZH: [&str; 10] = [
    "用手机解锁",
    "请在手机上用指纹确认。",
    "解锁",
    "手机上未确认。",
    "此账户未设置用手机解锁。请在这台电脑的 NectarLink 中打开。",
    "手机未连接。请靠近并连接同一网络。",
    "尝试次数过多。请使用密码，或稍后再试。",
    "NectarLink 未在此账户下运行。",
    "Windows 未接受该密码。如果你改过密码，请在 NectarLink 中重新设置解锁。",
    "无法用手机解锁。请使用密码。",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_language_has_every_text_and_unknown_ones_get_english() {
        for table in [&EN, &DE, &ES, &JA, &PL, &PT, &RU, &TR, &VI, &ZH] {
            assert!(table.iter().all(|s| !s.is_empty() && !s.contains('\u{2014}')));
        }
        assert_eq!(get(Text::Unlock, 0x09), "Unlock");
        assert_eq!(get(Text::Unlock, 0x7f), "Unlock");
        assert_eq!(get(Text::Unlock, 0x07), "Entsperren");
    }
}
