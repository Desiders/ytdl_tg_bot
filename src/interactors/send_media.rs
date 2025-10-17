mod fs;
mod id;

pub use fs::{SendAudioInFS, SendAudioInFSInput, SendVideoInFS, SendVideoInFSInput};
pub use id::{
    EditAudioById, EditAudioByIdInput, EditVideoById, EditVideoByIdInput, SendAudioById, SendAudioByIdInput, SendAudioPlaylistById,
    SendAudioPlaylistByIdInput, SendVideoById, SendVideoByIdInput, SendVideoPlaylistById, SendVideoPlaylistByIdInput,
};
