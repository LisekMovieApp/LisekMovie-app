//! Filename-parsing heuristics for common movie/TV release-naming
//! conventions (see the Faza 5 research notes). Split into a shared cleaning pass plus one
//! module per content type, since movies and episodes need materially
//! different marker patterns even though they share the same separator
//! normalization and title-tidying groundwork.

pub mod clean;
pub mod episode;
pub mod movie;

pub use episode::{EpisodePathContext, ParsedEpisode, parse_episode};
pub use movie::{ParsedMovie, parse_movie_name};
