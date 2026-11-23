// SPDX-License-Identifier: AGPL-3.0-or-later
//! Indonesian -> English for the pages customers see (FR-53). The interface is authored in Indonesian; when the visitor
//! picks English a middleware rewrites only the *text nodes* of public HTML pages with this table (never tags or attributes).

/// Longest phrases first so that "Kirim pesanan" wins over "Kirim".
const TABLE: &[(&str, &str)] = &[
    (
        "Pajak &amp; layanan dihitung saat pesanan dikirim",
        "Tax &amp; service are calculated when the order is sent",
    ),
    (
        "Sedang ramai. Pesanan mungkin lebih lama sekitar",
        "It is busy. Orders may take about",
    ),
    (
        "Pesanan Anda sudah masuk. Kasir akan segera mengonfirmasi",
        "Your order is in. The cashier will confirm shortly",
    ),
    (
        "Pesanan Anda siap. Silakan bayar di kasir",
        "Your order is ready. Please pay at the counter",
    ),
    (
        "Pesanan tidak dapat diproses. Silakan tanya kasir",
        "The order could not be processed. Please ask the cashier",
    ),
    (
        "Permintaan bill terkirim. Kasir akan segera datang.",
        "Bill requested. The cashier will be right with you.",
    ),
    (
        "Pelayan dipanggil. Mohon tunggu sebentar.",
        "A waiter has been called. Please wait a moment.",
    ),
    (
        "Kami menyimpan nama dan nomor HP Anda hanya untuk poin dan penawaran kafe ini. Anda bisa minta datanya dihapus kapan saja.",
        "We store your name and phone number only for this café's points and offers. You can ask for your data to be deleted at any time.",
    ),
    (
        "Saya setuju data saya disimpan untuk program loyalty",
        "I agree to my data being stored for the loyalty programme",
    ),
    (
        "Struk digital · simpan atau tangkap layar",
        "Digital receipt · save or take a screenshot",
    ),
    ("Menunggu konfirmasi kasir", "Waiting for cashier confirmation"),
    ("Pembayaran dilakukan di kasir", "Pay at the counter"),
    ("Silakan pesan langsung ke kasir", "Please order at the counter"),
    ("Kafe sedang tutup", "The café is closed"),
    ("Pesanan belum bisa dikirim", "Orders cannot be sent right now"),
    ("Bagaimana pesanan Anda?", "How was your order?"),
    ("Nilai tiap menu (opsional)", "Rate each item (optional)"),
    ("Komentar (opsional)", "Comment (optional)"),
    ("Terima kasih atas nilai", "Thanks for rating us"),
    ("Lihat struk digital", "View digital receipt"),
    ("Menu belum tersedia", "Menu not available"),
    ("di meja ini hari ini", "at this table today"),
    ("Belum dibayar:", "Unpaid:"),
    ("Belum ada pesanan", "No orders yet"),
    ("Belum ada item", "No items yet"),
    ("Sedang disiapkan", "Being prepared"),
    ("Sedang diproses", "Being prepared"),
    ("Siap diambil", "Ready for pickup"),
    ("Kirim pesanan", "Place order"),
    ("Tambah pesanan", "Add to order"),
    ("Panggil pelayan", "Call a waiter"),
    ("Gabung loyalty", "Join loyalty"),
    ("Tagihan meja", "Bill for table"),
    ("Layar antrean", "Queue display"),
    ("Pesanan #", "Order #"),
    ("Minta bill", "Ask for the bill"),
    ("Pesan lagi", "Order again"),
    ("Nomor HP", "Phone number"),
    ("jam buka", "opening hours"),
    ("Dibatalkan", "Cancelled"),
    ("Pembulatan", "Rounding"),
    ("Terima kasih", "Thank you"),
    ("Catatan", "Notes"),
    ("Selesai", "Done"),
    ("Ditolak", "Declined"),
    ("Opsional", "Optional"),
    ("opsional", "optional"),
    ("wajib", "required"),
    ("Tanpa", "None"),
    ("Habis", "Sold out"),
    ("Sisa ", "Left: "),
    ("Meja", "Table"),
    ("Siap", "Ready"),
    ("Nama", "Name"),
    ("Kirim", "Send"),
    ("Gabung", "Join"),
    ("Diskon", "Discount"),
    ("Pajak", "Tax"),
    ("Kembali", "Change"),
    ("Tunai", "Cash"),
    ("Struk", "Receipt"),
    ("Bayar", "Paid"),
    ("menit", "minutes"),
];

fn translate_text(text: &str) -> String {
    let mut out = text.to_string();
    for (id, en) in TABLE {
        if out.contains(id) {
            out = out.replace(id, en);
        }
    }
    out
}

/// Rewrite text nodes only; everything between `<` and `>` (tags, attributes, inline SVG) is copied verbatim.
/// Cuppa pages never contain inline `<script>`/`<style>` bodies (strict CSP), so there is nothing else to skip.
pub fn translate_html(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut text = String::new();
    let mut in_tag = false;
    for c in html.chars() {
        if in_tag {
            out.push(c);
            in_tag = c != '>';
        } else if c == '<' {
            out.push_str(&translate_text(&text));
            text.clear();
            out.push(c);
            in_tag = true;
        } else {
            text.push(c);
        }
    }
    out.push_str(&translate_text(&text));
    out.replacen("lang=\"id\"", "lang=\"en\"", 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translates_text_nodes_only() {
        let html = r#"<html lang="id"><a class="Siap" href="/m/Meja" aria-label="Kirim"><b>Kirim pesanan</b></a> Meja 3 · Siap</html>"#;
        let out = translate_html(html);
        assert!(out.contains("lang=\"en\""));
        assert!(out.contains("<b>Place order</b>"), "{out}");
        assert!(out.contains("Table 3 · Ready"), "{out}");
        // attributes are untouched
        assert!(
            out.contains(r#"class="Siap" href="/m/Meja" aria-label="Kirim""#),
            "{out}"
        );
    }

    #[test]
    fn longer_phrases_win() {
        assert_eq!(translate_text("Kirim pesanan"), "Place order");
        assert_eq!(translate_text("Kirim"), "Send");
    }
}
