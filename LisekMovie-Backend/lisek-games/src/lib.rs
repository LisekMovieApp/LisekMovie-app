//! Rules shared by the game API. Scores are recreational client submissions.
pub fn valid_score(key: &str, score: i32) -> bool {
    key == "flappy-bird" && (0..=100_000).contains(&score)
}
#[cfg(test)]
mod tests {
    #[test]
    fn validates_registered_game_and_score_bounds() {
        assert!(super::valid_score("flappy-bird",0));
        assert!(!super::valid_score("flappy-bird",-1));
        assert!(!super::valid_score("flappy-bird",100_001));
        assert!(!super::valid_score("unknown",1));
    }
}
