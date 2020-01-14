
extern crate rand;
use rand::Rng;
pub fn rnd_sz(range: usize) ->usize { 
    let mut rng = rand::thread_rng();
    (rng.gen::<usize>() % range)
  }