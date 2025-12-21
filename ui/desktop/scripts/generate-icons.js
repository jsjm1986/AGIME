/**
 * AGIME Icon Generator
 * Converts SVG logo to PNG, ICO and ICNS formats
 */

const fs = require('fs');
const path = require('path');

// Check if sharp is available
let sharp;
try {
  sharp = require('sharp');
} catch (e) {
  console.log('Installing sharp...');
  require('child_process').execSync('npm install sharp --save-dev', { stdio: 'inherit' });
  sharp = require('sharp');
}

// Check if to-ico is available
let toIco;
try {
  toIco = require('to-ico');
} catch (e) {
  console.log('Installing to-ico...');
  require('child_process').execSync('npm install to-ico --save-dev', { stdio: 'inherit' });
  toIco = require('to-ico');
}

// Check if png2icons is available
let png2icons;
try {
  png2icons = require('png2icons');
} catch (e) {
  console.log('Installing png2icons...');
  require('child_process').execSync('npm install png2icons --save-dev', { stdio: 'inherit' });
  png2icons = require('png2icons');
}

const IMAGES_DIR = path.join(__dirname, '..', 'src', 'images');
const SVG_PATH = path.join(IMAGES_DIR, 'agime-logo.svg');

// Icon sizes needed
const SIZES = [16, 32, 48, 64, 128, 256, 512, 1024];

async function generateIcons() {
  console.log('🎨 Generating AGIME icons...\n');

  // Read SVG
  const svgBuffer = fs.readFileSync(SVG_PATH);
  console.log(`✓ Read SVG from: ${SVG_PATH}`);

  // Generate PNGs at different sizes
  const pngBuffers = {};
  for (const size of SIZES) {
    const pngBuffer = await sharp(svgBuffer)
      .resize(size, size)
      .png()
      .toBuffer();

    pngBuffers[size] = pngBuffer;
    console.log(`✓ Generated ${size}x${size} PNG`);
  }

  // Save main icon.png (512px)
  const iconPngPath = path.join(IMAGES_DIR, 'icon.png');
  fs.writeFileSync(iconPngPath, pngBuffers[512]);
  console.log(`✓ Saved: ${iconPngPath}`);

  // Save icon@2x.png (1024px)
  const icon2xPath = path.join(IMAGES_DIR, 'icon@2x.png');
  fs.writeFileSync(icon2xPath, pngBuffers[1024]);
  console.log(`✓ Saved: ${icon2xPath}`);

  // Save icon-light.png (same as icon.png for now)
  const iconLightPath = path.join(IMAGES_DIR, 'icon-light.png');
  fs.writeFileSync(iconLightPath, pngBuffers[512]);
  console.log(`✓ Saved: ${iconLightPath}`);

  // Generate ICO (Windows) - includes multiple sizes
  const icoSizes = [16, 32, 48, 64, 128, 256];
  const icoBuffers = icoSizes.map(size => pngBuffers[size]);

  const icoBuffer = await toIco(icoBuffers);
  const icoPath = path.join(IMAGES_DIR, 'icon.ico');
  fs.writeFileSync(icoPath, icoBuffer);
  console.log(`✓ Saved: ${icoPath}`);

  // Generate ICNS (macOS) using png2icons
  console.log('\n📦 Generating macOS ICNS files...');

  // Use 1024x1024 PNG as source for ICNS
  const icnsBuffer = png2icons.createICNS(pngBuffers[1024], png2icons.BILINEAR, 0);
  if (icnsBuffer) {
    const icnsPath = path.join(IMAGES_DIR, 'icon.icns');
    fs.writeFileSync(icnsPath, icnsBuffer);
    console.log(`✓ Saved: ${icnsPath}`);

    // Also save icon-light.icns
    const icnsLightPath = path.join(IMAGES_DIR, 'icon-light.icns');
    fs.writeFileSync(icnsLightPath, icnsBuffer);
    console.log(`✓ Saved: ${icnsLightPath}`);
  } else {
    console.log('⚠️  Failed to generate ICNS file');
  }

  // Generate template icons for system tray (macOS)
  // These should be smaller and work well in menu bar
  const templatePath = path.join(IMAGES_DIR, 'iconTemplate.png');
  const template2xPath = path.join(IMAGES_DIR, 'iconTemplate@2x.png');

  // Template icons are typically 16px and 32px (for @2x)
  fs.writeFileSync(templatePath, pngBuffers[16]);
  fs.writeFileSync(template2xPath, pngBuffers[32]);
  console.log(`✓ Saved: ${templatePath}`);
  console.log(`✓ Saved: ${template2xPath}`);

  // Copy for update template icons
  const templateUpdatePath = path.join(IMAGES_DIR, 'iconTemplateUpdate.png');
  const templateUpdate2xPath = path.join(IMAGES_DIR, 'iconTemplateUpdate@2x.png');
  fs.writeFileSync(templateUpdatePath, pngBuffers[16]);
  fs.writeFileSync(templateUpdate2xPath, pngBuffers[32]);
  console.log(`✓ Saved: ${templateUpdatePath}`);
  console.log(`✓ Saved: ${templateUpdate2xPath}`);

  // Update icon.svg
  const iconSvgPath = path.join(IMAGES_DIR, 'icon.svg');
  fs.copyFileSync(SVG_PATH, iconSvgPath);
  console.log(`✓ Saved: ${iconSvgPath}`);

  console.log('\n✅ All icons generated successfully!');
}

generateIcons().catch(console.error);
