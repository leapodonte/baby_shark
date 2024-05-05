pub mod builder;

use self::fast_sweep::FastSweeping;
use self::visitors::value_mut_visitor::ValueMutVisitor;
use crate::voxel::*;
use crate::{dynamic_vdb, helpers::aliases::Vec3f};
use std::fs::File;
use std::marker::PhantomData;
use std::path::Path;
use volume::visitors::keep_sign_change_cubes::KeepSignChangeCubes;
use volume::visitors::min_max_index_visitor::MinMaxIdxVisitor;

pub(super) type VolumeGrid = dynamic_vdb!(f32, par 5, 4, 3);

#[derive(Debug)]
pub struct Volume {
    grid: Box<VolumeGrid>,
    voxel_size: f32,
}

impl Volume {
    #[inline]
    pub(super) fn new(grid: Box<VolumeGrid>, voxel_size: f32) -> Self {
        Self { grid, voxel_size }
    }

    ///
    /// Creates new SDF grid by evaluating given function on each grid point.
    /// Inside is negative.
    ///
    pub fn from_fn<TFn: Fn(&Vec3f) -> f32>(
        voxel_size: f32,
        min: Vec3f,
        max: Vec3f,
        narrow_band_width: usize,
        func: TFn,
    ) -> Self {
        let mut grid = VolumeGrid::empty(Vec3i::zeros());

        let narrow_band_width = (narrow_band_width + 1) as f32 * voxel_size;
        let min = (min / voxel_size).map(|x| x.floor() as isize);
        let max = (max / voxel_size).map(|x| x.ceil() as isize);

        for x in min.x..=max.x {
            for y in min.y..=max.y {
                for z in min.z..=max.z {
                    let idx = Vec3i::new(x, y, z);
                    let grid_point = idx.cast() * voxel_size;
                    let value = func(&grid_point);

                    if value.abs() > narrow_band_width {
                        continue;
                    }

                    grid.insert(&idx, value);
                }
            }
        }

        // TODO: prune

        Self { grid, voxel_size }
    }

    pub fn union(mut self, mut other: Self) -> Self {
        self.grid.flood_fill();
        other.grid.flood_fill();
        self.grid.union(other.grid);
        self
    }

    pub fn intersect(mut self, mut other: Self) -> Self {
        self.grid.flood_fill();
        other.grid.flood_fill();
        self.grid.intersect(other.grid);
        self
    }

    pub fn subtract(mut self, mut other: Self) -> Self {
        self.grid.flood_fill();
        other.grid.flood_fill();
        self.grid.subtract(other.grid);
        self
    }

    pub fn offset(mut self, distance: f32) -> Self {
        let mut iso_level = KeepSignChangeCubes::new(self.grid.as_ref());
        self.grid.visit_leafs(&mut iso_level);
        let new_grid = iso_level.sign_changes();
        self.grid = new_grid;

        let time = std::time::Instant::now();

        let mut extension_distance = distance.abs() + self.voxel_size + self.voxel_size;
        extension_distance.set_sign(distance.sign());

        let mut sweep = FastSweeping::new(self.voxel_size, extension_distance);
        sweep.fast_sweep(self.grid.as_mut());

        println!("Fast sweeping took: {:?}", time.elapsed());

        // volume_to_nrrd(&self.grid, Path::new("offset.nrrd"));

        let mut offset = ValueMutVisitor::<VolumeGrid, _>::from_fn(|v| *v -= distance);
        self.grid.visit_values_mut(&mut offset);

        self
    }

    pub(in crate::voxel) fn grid(&self) -> &VolumeGrid {
        // HIDE
        &self.grid
    }
}

impl Clone for Volume {
    fn clone(&self) -> Self {
        Self {
            grid: self.grid.clone(),
            voxel_size: self.voxel_size,
        }
    }
}

fn volume_to_nrrd(volume: &VolumeGrid, path: &Path) {
    use rusty_nrrd::*;

    let mut min_max_idx = MinMaxIdxVisitor::<VolumeGrid>::new();
    volume.visit_leafs(&mut min_max_idx);
    let MinMaxIdxVisitor { min, max, .. } = min_max_idx;

    println!("Min: {:?}, Max: {:?}", min, max);

    let min = Vec3i::new(-152, -120, -16);
    let max = Vec3i::new(152, 120, 272);

    let sizes = max - min + Vec3i::new(1, 1, 1);
    let background = 1000.0;
    let image = Image::<f32, 3>::new(
        background,
        [sizes.x as usize, sizes.y as usize, sizes.z as usize],
    );

    let mut visitor = VolumeToImage::<VolumeGrid> {
        image,
        min,
        background,
        _tree: PhantomData,
    };

    volume.visit_leafs(&mut visitor);

    let nrrd = Nrrd::try_from(&visitor.image).unwrap();
    write_nrrd(&nrrd, File::create(path).unwrap()).expect("write nrrd");
}

use rusty_nrrd::*;

struct VolumeToImage<T: TreeNode<Value = f32>> {
    image: Image<f32, 3>,
    min: Vec3i,
    background: f32,
    _tree: PhantomData<T>,
}

impl<T: TreeNode<Value = f32>> Visitor<T::Leaf> for VolumeToImage<T> {
    fn tile(&mut self, tile: Tile<<T::Leaf as TreeNode>::Value>) {
        for x in 0..tile.size {
            for y in 0..tile.size {
                for z in 0..tile.size {
                    // if tile.value.sign() == Sign::Positive {
                    //     self.image[[x, y, z]] = 1.0;
                    // } else {
                    //     self.image[[x, y, z]] = -1.0;
                    // }

                    self.image[[x, y, z]] = tile.value;
                }
            }
        }
    }

    fn dense(&mut self, dense: &T::Leaf) {
        for x in 0..T::Leaf::resolution() {
            for y in 0..T::Leaf::resolution() {
                for z in 0..T::Leaf::resolution() {
                    let idx = dense.origin() + Vec3i::new(x as isize, y as isize, z as isize);
                    let shifted = idx - self.min;
                    let shifted_usize = shifted.map(|x| x as usize);

                    let val = dense.at(&idx).copied();
                    // let val = dense.at(&idx).copied().map(|v| if v.sign() == Sign::Positive { 1.0 } else { -1.0 });

                    self.image[[shifted_usize.x, shifted_usize.y, shifted_usize.z]] =
                        val.unwrap_or(self.background);
                }
            }
        }
    }
}
