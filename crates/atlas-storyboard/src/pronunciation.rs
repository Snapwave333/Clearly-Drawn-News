use std::collections::HashMap;

/// Phonetic dictionary for Japanese animation studios, creators, directors,
/// and industry terms to ensure flawless TTS and phoneme alignment.
pub struct PronunciationDictionary {
    entries: HashMap<&'static str, (&'static str, &'static str)>, // (word, (phonetic_spelling, ipa))
}

impl Default for PronunciationDictionary {
    fn default() -> Self {
        Self::new()
    }
}

impl PronunciationDictionary {
    pub fn new() -> Self {
        let mut entries = HashMap::new();

        // Studios
        entries.insert("MAPPA", ("MAH-pah", "mɑːpə"));
        entries.insert("Ufotable", ("OO-foh-tay-bul", "uːfoʊteɪbəl"));
        entries.insert("ufotable", ("OO-foh-tay-bul", "uːfoʊteɪbəl"));
        entries.insert("Wit Studio", ("WIT STOO-dee-oh", "wɪt ˈstuːdioʊ"));
        entries.insert("Studio Ghibli", ("STOO-dee-oh JIB-lee", "ˈstuːdioʊ ˈdʒɪbli"));
        entries.insert("Ghibli", ("JIB-lee", "ˈdʒɪbli"));
        entries.insert("Science SARU", ("SY-ens SAH-roo", "ˈsaɪəns ˈsɑːruː"));
        entries.insert("Kyoto Animation", ("kee-OH-toh AN-ih-MAY-shun", "kjoʊtoʊ ˌænɪˈmeɪʃən"));
        entries.insert("KyoAni", ("kyoh-AH-nee", "kjoʊˈɑːni"));
        entries.insert("Bones", ("BOHNZ", "boʊnz"));
        entries.insert("CloverWorks", ("KLOH-ver-wurks", "ˈkloʊvərˌwɜːrks"));
        entries.insert("Madhouse", ("MAD-howss", "ˈmædˌhaʊs"));
        entries.insert("Trigger", ("TRIG-er", "ˈtrɪɡər"));
        entries.insert("Production I.G", ("proh-DUK-shun EYE-JEE", "prəˈdʌkʃən aɪ dʒiː"));
        entries.insert("Fortiche", ("for-TEESH", "fɔːrˈtiːʃ"));
        entries.insert("CoMix Wave Films", ("KOH-miks WAYV FILMZ", "ˈkoʊmɪks weɪv fɪlmz"));

        // Creators & Directors
        entries.insert("Hayao Miyazaki", ("HAH-yah-oh mee-yah-ZAH-kee", "hɑːjɑːoʊ miːjəˈzɑːki"));
        entries.insert("Miyazaki", ("mee-yah-ZAH-kee", "miːjəˈzɑːki"));
        entries.insert("Makoto Shinkai", ("mah-KOH-toh SHIN-kye", "mɑːˈkoʊtoʊ ʃɪnkaɪ"));
        entries.insert("Shinkai", ("SHIN-kye", "ʃɪnkaɪ"));
        entries.insert("Hideaki Anno", ("hee-deh-AH-kee AHN-noh", "hiːdeɪˈɑːki ˈɑːnoʊ"));
        entries.insert("Anno", ("AHN-noh", "ˈɑːnoʊ"));
        entries.insert("Satoshi Kon", ("sah-TOH-shee KOHN", "sɑːˈtoʊʃi koʊn"));
        entries.insert("Tatsuki Fujimoto", ("taht-SOO-kee foo-jee-MOH-toh", "tɑːtˈsuːki fuːdʒiːˈmoʊtoʊ"));
        entries.insert("Fujimoto", ("foo-jee-MOH-toh", "fuːdʒiːˈmoʊtoʊ"));
        entries.insert("Sunghoo Park", ("SUNG-hoo PARK", "sʌŋhuː pɑːrk"));
        entries.insert("Kensuke Ushio", ("KEN-soo-kay OO-shee-oh", "ˈkɛnsuːkeɪ ˈuːʃioʊ"));
        entries.insert("Fuga Yamashiro", ("FOO-gah yah-mah-SHEE-roh", "ˈfuːɡɑː jɑːmɑːˈʃiːroʊ"));

        // Shows & Properties
        entries.insert("Dandadan", ("DAHN-dah-dahn", "ˈdɑːndɑːdɑːn"));
        entries.insert("Jujutsu Kaisen", ("joo-JOOT-soo KYE-zen", "dʒuːˈdʒuːtsuː ˈkaɪzɛn"));
        entries.insert("Chainsaw Man", ("CHAYN-saw MAN", "ˈtʃeɪnsɔː mæn"));
        entries.insert("Sousou no Frieren", ("SOH-soh noh FREE-ren", "soʊsoʊ noʊ ˈfriːrɛn"));
        entries.insert("Frieren", ("FREE-ren", "ˈfriːrɛn"));
        entries.insert("Demon Slayer", ("DEE-mun SLAY-er", "ˈdiːmən ˈsleɪər"));
        entries.insert("Kimetsu no Yaiba", ("kee-MET-soo noh YYE-bah", "kiːˈmɛtsuː noʊ ˈjaɪbə"));
        entries.insert("Arcane", ("ar-KAYN", "ɑːrˈkeɪn"));
        entries.insert("Spider-Verse", ("SPY-der-vurss", "ˈspaɪdər vɜːrs"));

        Self { entries }
    }

    pub fn phonetically_prepare(&self, text: &str) -> String {
        let mut terms: Vec<(&&str, &(&str, &str))> = self.entries.iter().collect();
        terms.sort_by_key(|(term, _)| std::cmp::Reverse(term.len()));
        let mut prepared = text.to_string();
        for (term, (phonetic, _)) in terms {
            if prepared.contains(*term) {
                prepared = prepared.replace(*term, &phonetic.to_lowercase());
            }
        }
        prepared
    }

    /// Returns the IPA phoneme if present
    pub fn get_ipa(&self, term: &str) -> Option<&'static str> {
        self.entries.get(term).map(|(_, ipa)| *ipa)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pronunciation_guide() {
        let dict = PronunciationDictionary::new();
        let sample = "MAPPA and Studio Ghibli announced a collaboration with Tatsuki Fujimoto.";
        let phon = dict.phonetically_prepare(sample);
        assert!(phon.contains("mah-pah"));
        assert!(phon.contains("stoo-dee-oh jib-lee"));
    }
}
