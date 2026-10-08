// SPDX-License-Identifier: MPL-2.0
//! Smart one-time code (OTP) detection on notification and SMS text.
//!
//! Codes are detected locally and never logged or stored in clipboard history.

/// Extracts a 4–8 digit one-time code (or prefixed/hyphenated format like
/// `"G-123456"` or `"123-456"`) when it appears near an OTP keyword in English
/// or common languages, while ignoring dates, times, currency amounts, phone
/// numbers, and order/card/account numbers.
pub fn one_time_code(body: &str) -> Option<String> {
    if body.is_empty() {
        return None;
    }
    let chars: Vec<char> = body.chars().collect();
    // One lowercase char per original char, so indices line up between the
    // two ("İ" lowercases to two chars).
    let lower_chars: Vec<char> = chars.iter().map(|c| c.to_lowercase().next().unwrap_or(*c)).collect();
    let keyword_spans = find_keyword_spans(&lower_chars);
    if keyword_spans.is_empty() {
        return None;
    }

    let candidates = find_candidates(&chars, &lower_chars);

    candidates
        .into_iter()
        .filter_map(|cand| {
            let dist = keyword_spans
                .iter()
                .map(|&(ks, ke)| if cand.end <= ks { ks - cand.end } else { cand.start.saturating_sub(ke) })
                .min()?;
            // Must be reasonably close to an OTP keyword.
            if dist > 100 {
                return None;
            }
            Some((dist, cand.code))
        })
        .min_by_key(|(dist, _)| *dist)
        .map(|(_, code)| code)
}

/// Whether `text` is a bare one-time code or a message containing one,
/// so clipboard history can exclude it.
pub fn is_otp_clip(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return false;
    }
    if is_bare_code(trimmed) {
        return true;
    }
    one_time_code(trimmed).is_some()
}

/// Whether `trimmed` is nothing but a standalone 4–8 digit code, `"G-123456"`,
/// `"123-456"` or `"1234-5678"`.
pub fn is_bare_code(trimmed: &str) -> bool {
    if (4..=8).contains(&trimmed.len()) && trimmed.bytes().all(|b| b.is_ascii_digit()) {
        return true;
    }
    if let Some((prefix, rest)) = trimmed.split_once('-') {
        if (1..=3).contains(&prefix.len())
            && prefix.bytes().all(|b| b.is_ascii_alphabetic())
            && (4..=8).contains(&rest.len())
            && rest.bytes().all(|b| b.is_ascii_digit())
        {
            return true;
        }
        if (prefix.len() == 3 || prefix.len() == 4)
            && rest.len() == prefix.len()
            && prefix.bytes().all(|b| b.is_ascii_digit())
            && rest.bytes().all(|b| b.is_ascii_digit())
        {
            return true;
        }
    }
    false
}

#[derive(Debug)]
struct Candidate {
    start: usize,
    end: usize,
    code: String,
}

/// Word-bounded keywords (matched as whole words in `lower_chars`).
const WORD_KEYWORDS: &[&str] = &[
    // English
    "otp",
    "code",
    "codes",
    "passcode",
    "password",
    "verification",
    "verify",
    "pin",
    "login",
    "signin",
    "2fa",
    "mfa",
    "authentication",
    "authenticator",
    // Spanish / Portuguese / Italian
    "código",
    "codigo",
    "verificación",
    "verificacion",
    "verificação",
    "verificacao",
    "acceso",
    "acesso",
    "clave",
    "senha",
    "codice",
    "verifica",
    "accesso",
    "autenticación",
    "autenticacao",
    "autenticazione",
    // French
    "vérification",
    "connexion",
    "authentification",
    // German / Dutch
    "bestätigungscode",
    "bestatigungscode",
    "bestätigung",
    "einmalpasswort",
    "sicherheitscode",
    "anmeldung",
    "verifizierung",
    "zugangscode",
    "prüfcode",
    "verificatiecode",
    "beveiligingscode",
    "inlogcode",
    "eenmalig",
    // Turkish
    "doğrulama",
    "dogrulama",
    // Russian / Ukrainian
    "код",
    "подтверждения",
    "проверочный",
    "одноразовый",
    "пароль",
    "підтвердження",
    // Arabic
    "رمز",
    "تحقق",
    "التحقق",
    "كود",
    "تأكيد",
    // Hindi
    "कोड",
    "ओटीपी",
    "सत्यापन",
    "पासकोड",
    "पिन",
    "लॉगिन",
];

/// Multi-word phrases and CJK substrings.
const PHRASE_KEYWORDS: &[&str] = &[
    "one-time",
    "one time",
    "sign in",
    "sign-in",
    "log in",
    "log-in",
    "two-factor",
    "two factor",
    "security code",
    "confirmation code",
    "access code",
    "temporary password",
    "iniciar sesión",
    "iniciar sesion",
    "mot de passe",
    "einmal-passwort",
    "doğrulama kodu",
    "dogrulama kodu",
    "onay kodu",
    "güvenlik kodu",
    // Chinese (Simplified & Traditional)
    "验证码",
    "驗證碼",
    "校验码",
    "校驗碼",
    "动态码",
    "動態碼",
    "安全码",
    "安全碼",
    "登录码",
    "確認碼",
    "一次性密码",
    // Japanese
    "認証コード",
    "確認コード",
    "ワンタイムパスワード",
    "セキュリティコード",
    "暗証番号",
    "認証番号",
    "コード",
    // Korean
    "인증번호",
    "인증 번호",
    "확인코드",
    "확인 코드",
    "보안코드",
    "보안 코드",
    "일회용 비밀번호",
    "핀번호",
];

/// Words immediately preceding `"code"` / `"codes"` that mean a non-OTP code.
const NON_OTP_CODE_PREFIXES: &[&str] = &[
    "zip",
    "postal",
    "area",
    "country",
    "promo",
    "promotion",
    "promotional",
    "discount",
    "coupon",
    "voucher",
    "referral",
    "invite",
    "bar",
    "qr",
    "dress",
    "source",
    "error",
    "status",
    "fault",
    "tracking",
    "sort",
    "swift",
    "bic",
    "ifsc",
    "hs",
    "color",
    "colour",
    "product",
    "item",
    "tax",
];

fn find_keyword_spans(lower_chars: &[char]) -> Vec<(usize, usize)> {
    let mut spans = Vec::new();

    // Phrase / CJK matches.
    for &phrase in PHRASE_KEYWORDS {
        let phrase_chars: Vec<char> = phrase.chars().collect();
        let plen = phrase_chars.len();
        if plen == 0 || lower_chars.len() < plen {
            continue;
        }
        let ascii_word = phrase_chars.first().is_some_and(char::is_ascii_alphanumeric);
        for i in 0..=(lower_chars.len() - plen) {
            if lower_chars[i..i + plen] == phrase_chars[..] {
                if ascii_word {
                    let before_ok = i == 0 || !lower_chars[i - 1].is_alphanumeric();
                    let after_ok = i + plen == lower_chars.len() || !lower_chars[i + plen].is_alphanumeric();
                    if !before_ok || !after_ok {
                        continue;
                    }
                }
                spans.push((i, i + plen));
            }
        }
    }

    // Tokenize into words with char indices.
    let mut i = 0;
    let mut prev_word: Option<(usize, usize, String)> = None;
    while i < lower_chars.len() {
        if !lower_chars[i].is_alphanumeric() {
            i += 1;
            continue;
        }
        let start = i;
        while i < lower_chars.len() && lower_chars[i].is_alphanumeric() {
            i += 1;
        }
        let word: String = lower_chars[start..i].iter().collect();
        if WORD_KEYWORDS.contains(&word.as_str()) {
            let is_non_otp_code = (word == "code" || word == "codes")
                && prev_word.as_ref().is_some_and(|(_, pe, pw)| {
                    start.saturating_sub(*pe) <= 2 && NON_OTP_CODE_PREFIXES.contains(&pw.as_str())
                });
            if !is_non_otp_code {
                spans.push((start, i));
            }
        }
        prev_word = Some((start, i, word));
    }

    spans
}

fn find_candidates(chars: &[char], lower_chars: &[char]) -> Vec<Candidate> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        // Check for prefixed format like "G-123456" or "FB-12345":
        // 1..=3 ASCII uppercase letters + '-' + 4..=8 ASCII digits.
        if chars[i].is_ascii_uppercase() {
            let before_ok = i == 0 || !chars[i - 1].is_alphanumeric();
            if before_ok {
                let mut p_end = i;
                while p_end < chars.len() && chars[p_end].is_ascii_uppercase() && p_end - i <= 3 {
                    p_end += 1;
                }
                let p_len = p_end - i;
                if (1..=3).contains(&p_len)
                    && chars.get(p_end) == Some(&'-')
                    && chars.get(p_end + 1).is_some_and(char::is_ascii_digit)
                {
                    let d_start = p_end + 1;
                    let mut d_end = d_start;
                    while d_end < chars.len() && chars[d_end].is_ascii_digit() {
                        d_end += 1;
                    }
                    let d_len = d_end - d_start;
                    let after_ok = is_valid_after(chars, lower_chars, d_end);
                    if (4..=8).contains(&d_len)
                        && after_ok
                        && !is_excluded_context(chars, lower_chars, i, d_end)
                    {
                        out.push(Candidate {
                            start: i,
                            end: d_end,
                            code: chars[d_start..d_end].iter().collect(),
                        });
                        i = d_end;
                        continue;
                    }
                }
            }
        }

        if !chars[i].is_ascii_digit() {
            i += 1;
            continue;
        }

        let start = i;
        while i < chars.len() && chars[i].is_ascii_digit() {
            i += 1;
        }
        let len = i - start;

        // Grouped codes, "123-456" or "1234-5678" (not part of a longer
        // hyphenated chain like a phone number).
        if (len == 3 || len == 4)
            && chars.get(i) == Some(&'-')
            && chars.get(i + 1).is_some_and(char::is_ascii_digit)
        {
            let d2_start = i + 1;
            let mut d2_end = d2_start;
            while d2_end < chars.len() && chars[d2_end].is_ascii_digit() {
                d2_end += 1;
            }
            if d2_end - d2_start == len
                && is_valid_before(chars, start)
                && is_valid_after(chars, lower_chars, d2_end)
                && chars.get(d2_end) != Some(&'-')
                && !is_excluded_context(chars, lower_chars, start, d2_end)
            {
                let mut code: String = chars[start..i].iter().collect();
                code.extend(chars[d2_start..d2_end].iter());
                out.push(Candidate { start, end: d2_end, code });
                i = d2_end;
                continue;
            }
        }

        if (4..=8).contains(&len)
            && is_valid_before(chars, start)
            && is_valid_after(chars, lower_chars, i)
            && !is_excluded_context(chars, lower_chars, start, i)
        {
            out.push(Candidate { start, end: i, code: chars[start..i].iter().collect() });
        }
    }
    out
}

fn is_valid_before(chars: &[char], start: usize) -> bool {
    let Some(prev) = start.checked_sub(1).map(|idx| chars[idx]) else {
        return true;
    };
    if prev.is_alphanumeric() || is_currency_symbol(prev) || matches!(prev, '.' | ',' | '+') {
        return false;
    }
    // Date / time / decimal separator immediately before when preceded by a digit (e.g. "10/08/2026", "12:3000").
    if matches!(prev, '/' | ':') && start >= 2 && chars[start - 2].is_ascii_digit() {
        return false;
    }
    true
}

fn is_valid_after(chars: &[char], lower_chars: &[char], end: usize) -> bool {
    let Some(&next) = chars.get(end) else {
        return true;
    };
    if next.is_alphanumeric() || is_currency_symbol(next) || next == '%' {
        return false;
    }
    // Formatted number, decimal, date, or time ("1,234", "5000.00", "2026/10/08", "1234:56").
    if matches!(next, '.' | ',' | '/' | ':') && chars.get(end + 1).is_some_and(char::is_ascii_digit) {
        return false;
    }
    // Check if followed by whitespace + currency symbol/code or time suffix.
    let mut k = end;
    while k < chars.len() && (chars[k] == ' ' || chars[k] == '\t') {
        k += 1;
    }
    if k > end && k < chars.len() {
        if is_currency_symbol(chars[k]) || chars[k] == '%' {
            return false;
        }
        if lower_chars[k].is_alphabetic() {
            let w_start = k;
            let mut w_end = k;
            while w_end < lower_chars.len() && lower_chars[w_end].is_alphabetic() {
                w_end += 1;
            }
            let word: String = lower_chars[w_start..w_end].iter().collect();
            if matches!(
                word.as_str(),
                "usd"
                    | "eur"
                    | "gbp"
                    | "inr"
                    | "jpy"
                    | "cad"
                    | "aud"
                    | "chf"
                    | "cny"
                    | "dollars"
                    | "dollar"
                    | "euros"
                    | "euro"
                    | "pounds"
                    | "pound"
                    | "rupees"
                    | "rupee"
                    | "yen"
                    | "yuan"
                    | "hrs"
                    | "am"
                    | "pm"
            ) {
                return false;
            }
        }
    }
    true
}

fn is_currency_symbol(c: char) -> bool {
    matches!(c, '₹' | '$' | '€' | '£' | '¥' | '₩' | '₽' | '₺' | '₫' | '฿' | '₴' | '₦' | '₱' | '₪')
}

/// Checks broader context around `[start..end]` to reject dates, phone numbers,
/// currency amounts with spaces ("Rs. 5000"), and order/card/account numbers.
fn is_excluded_context(chars: &[char], lower_chars: &[char], start: usize, end: usize) -> bool {
    let len = end - start;

    // 1. Date patterns like YYYY-MM-DD or DD-MM-YYYY.
    if len == 4 {
        // YYYY-MM-DD: followed by '-' + 1..=2 digits + '-' + 1..=2 digits.
        if chars.get(end) == Some(&'-') {
            let m_start = end + 1;
            let mut m_end = m_start;
            while m_end < chars.len() && chars[m_end].is_ascii_digit() {
                m_end += 1;
            }
            if (1..=2).contains(&(m_end - m_start)) && chars.get(m_end) == Some(&'-') {
                return true;
            }
        }
        // DD-MM-YYYY or MM-DD-YYYY: preceded by 1..=2 digits + '-'.
        if start >= 2 && chars[start - 1] == '-' {
            let mut d_start = start - 1;
            while d_start > 0 && chars[d_start - 1].is_ascii_digit() {
                d_start -= 1;
            }
            let d_len = (start - 1) - d_start;
            if (1..=2).contains(&d_len) && d_start >= 2 && chars[d_start - 1] == '-' {
                return true;
            }
        }
    }

    // 2. Phone number tail: e.g. "555-0100", "(555) 0100", "555 0100", "+1 800 555 0199".
    if start > 0 {
        let mut b = start;
        while b > 0 && matches!(chars[b - 1], ' ' | '-' | ')' | '.') {
            b -= 1;
        }
        // If there were separators (including '-' or ')' or spaces after '+') and digits right before them:
        let sep_slice = &chars[b..start];
        let has_phone_sep = sep_slice.contains(&'-') || sep_slice.contains(&')');
        let mut d_start = b;
        while d_start > 0 && chars[d_start - 1].is_ascii_digit() {
            d_start -= 1;
        }
        let prev_digits = b - d_start;
        if (2..=5).contains(&prev_digits) {
            if has_phone_sep {
                return true;
            }
            // Space-separated phone number like "+44 7911 123456" or "020 7946 0958".
            let before_prev = d_start.checked_sub(1).map(|idx| chars[idx]);
            if before_prev == Some('+') || before_prev == Some('(') || chars.get(d_start) == Some(&'0') {
                return true;
            }
        }
    }
    // Phone number head: e.g. "+1 5550100" where '+' precedes after spaces.
    {
        let mut b = start;
        while b > 0 && chars[b - 1] == ' ' {
            b -= 1;
        }
        if b > 0 && chars[b - 1] == '+' {
            return true;
        }
    }

    // 3. Currency symbol or prefix with space/dot before the number: e.g. "Rs. 5000", "$ 1200", "USD 4500".
    {
        let mut b = start;
        while b > 0 && matches!(chars[b - 1], ' ' | '\t' | '.') {
            b -= 1;
        }
        if b > 0 && is_currency_symbol(chars[b - 1]) {
            return true;
        }
        let mut w_start = b;
        while w_start > 0 && lower_chars[w_start - 1].is_alphabetic() {
            w_start -= 1;
        }
        if w_start < b && (w_start == 0 || !lower_chars[w_start - 1].is_alphanumeric()) {
            let word: String = lower_chars[w_start..b].iter().collect();
            if matches!(
                word.as_str(),
                "rs" | "inr" | "usd" | "eur" | "gbp" | "jpy" | "cad" | "aud" | "chf" | "cny" | "brl" | "mxn"
            ) {
                return true;
            }
        }
    }

    // 4. Preceding '#' (e.g. "#123456" or "order #123456"), unless preceded by an OTP word ("code #123456").
    if start > 0 && chars[start - 1] == '#' {
        let mut b = start - 1;
        while b > 0 && chars[b - 1] == ' ' {
            b -= 1;
        }
        let mut w_start = b;
        while w_start > 0 && lower_chars[w_start - 1].is_alphanumeric() {
            w_start -= 1;
        }
        let word: String = lower_chars[w_start..b].iter().collect();
        if !matches!(word.as_str(), "code" | "otp" | "pin" | "passcode") {
            return true;
        }
    }

    // 5. Inspect up to 3 words immediately before `start` within the same clause.
    let prev_words = preceding_clause_words(lower_chars, start, 3);
    for (idx, w) in prev_words.iter().enumerate() {
        let is_generic_code =
            matches!(w.as_str(), "code" | "código" | "codigo" | "codice" | "kod" | "kodu" | "код" | "कोड");
        if is_generic_code {
            if prev_words.get(idx + 1).is_some_and(|prefix| NON_OTP_CODE_PREFIXES.contains(&prefix.as_str()))
            {
                return true;
            }
            break;
        }
        // Stop if we hit an OTP keyword closer to the number than any non-OTP label.
        if WORD_KEYWORDS.contains(&w.as_str()) {
            break;
        }
        if matches!(
            w.as_str(),
            "order"
                | "pedido"
                | "commande"
                | "bestellung"
                | "ordine"
                | "tracking"
                | "shipment"
                | "delivery"
                | "invoice"
                | "receipt"
                | "transaction"
                | "txn"
                | "ref"
                | "reference"
                | "account"
                | "acct"
                | "card"
                | "ending"
                | "terminada"
                | "ticket"
                | "item"
                | "sku"
                | "ext"
                | "extension"
                | "call"
                | "dial"
                | "phone"
                | "tel"
        ) {
            return true;
        }
        // Month names before a 4-digit year ("Oct 2026", "October 2026").
        if len == 4
            && matches!(
                w.as_str(),
                "jan"
                    | "january"
                    | "feb"
                    | "february"
                    | "mar"
                    | "march"
                    | "apr"
                    | "april"
                    | "may"
                    | "jun"
                    | "june"
                    | "jul"
                    | "july"
                    | "aug"
                    | "august"
                    | "sep"
                    | "sept"
                    | "september"
                    | "oct"
                    | "october"
                    | "nov"
                    | "november"
                    | "dec"
                    | "december"
            )
        {
            return true;
        }
    }

    false
}

/// Collects up to `max_words` words before `pos` without crossing sentence punctuation.
fn preceding_clause_words(lower_chars: &[char], pos: usize, max_words: usize) -> Vec<String> {
    let mut words = Vec::new();
    let mut i = pos;
    let limit = pos.saturating_sub(32);
    while i > limit && words.len() < max_words {
        while i > limit && !lower_chars[i - 1].is_alphanumeric() {
            if matches!(lower_chars[i - 1], '.' | '!' | '?' | ';' | '\n' | '\r') {
                return words;
            }
            i -= 1;
        }
        if i <= limit {
            break;
        }
        let w_end = i;
        while i > 0 && lower_chars[i - 1].is_alphanumeric() {
            i -= 1;
        }
        words.push(lower_chars[i..w_end].iter().collect());
    }
    words
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_real_world_positive_examples() {
        let cases: &[(&str, &str)] = &[
            // English standard patterns
            ("Your verification code is 482913", "482913"),
            ("Dear Customer,\n243928 is your one time password (OTP).", "243928"),
            ("840941 is your OTP to create JioID", "840941"),
            ("740421 is OTP for Aadhaar (XX3465) valid for 10 mins", "740421"),
            ("Use 9182 to verify your account.", "9182"),
            ("Your login passcode: 12345678. Do not share it.", "12345678"),
            ("Use PIN 7391 to sign in to your workspace.", "7391"),
            ("Sign in with one-time code 582019", "582019"),
            ("Your 2FA code for GitHub is 394821.", "394821"),
            ("Enter 601928 to log in to Nectarlink.", "601928"),
            // Prefixed & hyphenated formats
            ("G-482193 is your Google verification code.", "482193"),
            ("G-123456 is your Google verification code", "123456"),
            ("FB-84920 is your Facebook confirmation code", "84920"),
            ("Your Stripe verification code is 482-913.", "482913"),
            ("Use code 1234-5678", "12345678"),
            ("İstanbul: doğrulama kodunuz 482913", "482913"),
            // Disambiguation when dates, amounts, phone numbers, card/order numbers also appear
            ("On 2026-10-08, your verification code is 482913.", "482913"),
            ("Order #849201: use verification code 591024 at pickup.", "591024"),
            ("Transaction of Rs. 5000 on card ending 4321. Your OTP is 908172.", "908172"),
            ("Payment of $1200 to Acct 9988: enter passcode 445566 to authorize.", "445566"),
            ("If you did not request login code 674829, call +1 800-555-0199.", "674829"),
            ("Call 555-0100 if you didn't request verification code 314159.", "314159"),
            // Common languages (Spanish, French, German, Portuguese, Italian, Russian, Hindi, Chinese, Japanese, Korean, Arabic)
            ("Tu código de verificación es 583920. No lo compartas.", "583920"),
            ("Votre code de vérification est 918273.", "918273"),
            ("Ihr Bestätigungscode lautet 736451.", "736451"),
            ("Seu código de verificação é 394810.", "394810"),
            ("Il tuo codice di verifica è 604928.", "604928"),
            ("Ваш код подтверждения: 551902", "551902"),
            ("आपका ओटीपी 839201 है। किसी के साथ साझा न करें।", "839201"),
            ("【服务】您的验证码为 492018，请在5分钟内使用。", "492018"),
            ("認証コードは 384920 です。", "384920"),
            ("[안내] 인증번호 [849201]을 입력해주세요.", "849201"),
            ("رمز التحقق الخاص بك هو 748291", "748291"),
        ];

        for &(input, expected) in cases {
            assert_eq!(one_time_code(input).as_deref(), Some(expected), "expected {expected} from: {input}");
        }
    }

    #[test]
    fn rejects_real_world_negative_examples() {
        let cases: &[&str] = &[
            // No OTP keywords
            "Meet me at 1430 on 5th Avenue",
            "Payment received for Rs. 2000 - thanks",
            "Your package 482913 has been delivered!",
            "Call me back at 5550100 when you can",
            // Amounts & prices even when "code" or "pin" is in the text
            "Your code is ready. Pay Rs.5000 at the counter",
            "Your code is ready. Pay Rs. 5000 at the counter",
            "Use your PIN at checkout to pay $1250.",
            "Verification complete: €4500 transferred.",
            "PIN 1,234 used",
            "Code applied: save 5000.00 INR today",
            // Dates & times
            "Your login was recorded on 2026-10-08 at 12:30.",
            "Verification completed on 10/08/2026.",
            "Your password was changed in October 2026.",
            // Phone numbers
            "To reset your password, call 555-0100 or +1 800-555-0199.",
            "For verification support, dial +44 7911 123456.",
            // Order, tracking, account, card numbers
            "Order #482910 has shipped! Use your tracking code at the link.",
            "Your tracking code is 84920184 for order 192837.",
            "Login alert for card ending 4821.",
            "Verification requested for account 948201.",
            // Non-OTP "code" compounds (zip code, promo code, error code)
            "Enter promo code 2026 at checkout!",
            "Delivery to zip code 94043 is scheduled.",
            "Login failed with error code 4040.",
            // Too short, too long, or letters stuck to digits
            "OTP 12",
            "Your verification code is 123",
            "Your verification code is 123456789",
            "Your code is AB4821CD",
        ];

        for &input in cases {
            assert_eq!(one_time_code(input), None, "expected None from negative example: {input}");
        }
    }

    #[test]
    fn identifies_otp_clips_for_history_exclusion() {
        assert!(is_otp_clip("482913"));
        assert!(is_otp_clip(" G-482913 "));
        assert!(is_otp_clip("482-913"));
        assert!(is_otp_clip("1234-5678"));
        assert!(!is_otp_clip("2026-10-08"));
        assert!(is_otp_clip("Your verification code is 482913"));
        assert!(!is_otp_clip("hello from phone"));
        assert!(!is_otp_clip("https://example.com/path/123456"));
    }
}
